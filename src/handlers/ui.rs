use crate::{
    handlers::service_catalog::{
        build_service_sample_chat_title, build_service_sample_prompt, service_sample_ui_config,
    },
    handlers::upload::get_or_create_session,
    middleware::auth::auth_middleware,
    models::auth::Claims,
    AppState,
};
use axum::{
    extract::{Extension, Path, Query},
    response::Html,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn ui_routes() -> Router {
    Router::new()
        .route("/", get(landing_page))
        .route("/services", get(services_overview_page))
        .route("/services/saas-launch-pack", get(saas_launch_pack_page))
        .route(
            "/services/thumbnail-hero-pack",
            get(thumbnail_hero_pack_page),
        )
        .route(
            "/services/product-mockup-pack",
            get(product_mockup_pack_page),
        )
        .route(
            "/services/education-explainer-pack",
            get(education_explainer_pack_page),
        )
        .route("/services/blender-scene-pack", get(blender_scene_pack_page))
        .route("/services/voice-audio-pack", get(voice_audio_pack_page))
        .route(
            "/services/mixed-agency-bundle",
            get(mixed_agency_bundle_page),
        )
        .route(
            "/services/clipper-enhancement-pack",
            get(clipper_enhancement_pack_page),
        )
        .route(
            "/services/creator-manager-fulfillment",
            get(creator_manager_fulfillment_page),
        )
        .route("/services/x402-asset-api", get(x402_asset_api_page))
        .route(
            "/services/kick-com-clipping",
            get(kick_com_clipping_page),
        )
        .route(
            "/services/kick-auto-clipper",
            get(kick_com_clipping_page),
        )
        .route("/services/business-explainer-pack", get(saas_launch_pack_page))
        .route(
            "/services/manim-explainer",
            get(manim_explainer_page),
        )
        .route("/login", get(login_page))
        .route("/signup", get(signup_page))
        .route("/dashboard", get(dashboard_page))
        .route("/analytics", get(analytics_dashboard_page))
        .route("/clipping/manage", get(clipping_management_page))
        .route("/help", get(help_guide_page))
        .route("/privacy", get(privacy_policy_page))
        .route("/terms", get(terms_of_service_page))
        .route("/chat", get(chat_interface))
        .route("/chat/:session_id", get(chat_interface_with_session))
        .route("/app", get(chat_interface)) // Alternative route
        .route("/video-tools", get(video_tools_page))
        .route("/manual-clipping", get(manual_clipping_page))
        .route("/signup/clipper", get(clipper_signup_page))
}

pub fn ui_private_routes() -> Router {
    Router::new()
        .route("/api/service-samples/quota", get(get_service_sample_quota))
        .route(
            "/api/service-samples/request",
            post(create_service_sample_request),
        )
        .route("/campaigns", get(campaigns_list_page))
        .route("/campaigns/new", get(campaigns_new_page))
        .route("/campaigns/:id", get(campaigns_detail_page))
        .layer(axum::middleware::from_fn(
            crate::middleware::subscription::subscription_middleware,
        ))
        .layer(axum::middleware::from_fn(auth_middleware))
}

#[derive(Debug, Deserialize)]
pub struct ServiceSampleQuotaQuery {
    pub service: String,
}

#[derive(Debug, Deserialize)]
pub struct ServiceSampleRequest {
    pub service_slug: String,
    pub reference_url: Option<String>,
    pub prospect_name: Option<String>,
    pub brief: String,
    pub source: Option<String>,
}

fn service_sample_free_limit() -> i64 {
    std::env::var("SERVICE_SAMPLE_FREE_LIMIT")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(5)
}

fn parse_claim_user_id(claims: &Claims) -> Result<i32, String> {
    claims
        .sub
        .parse::<i32>()
        .map_err(|_| "Invalid user id in auth token".to_string())
}

pub async fn get_service_sample_quota(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    axum::extract::Query(query): axum::extract::Query<ServiceSampleQuotaQuery>,
) -> Json<serde_json::Value> {
    let user_id = match parse_claim_user_id(&claims) {
        Ok(id) => id,
        Err(message) => return Json(json!({"success": false, "message": message})),
    };

    let is_unlimited = claims.is_staff || claims.is_superuser;
    let limit = service_sample_free_limit();

    let used = if is_unlimited {
        0
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM service_sample_requests WHERE user_id = $1 AND source = 'videosync_service'",
        )
        .bind(user_id)
        .fetch_one(&state.db_pool)
        .await
        .unwrap_or(0)
    };

    Json(json!({
        "success": true,
        "service": query.service,
        "limit": limit,
        "used": used,
        "remaining": if is_unlimited { limit } else { std::cmp::max(0, limit - used) },
        "unlimited": is_unlimited
    }))
}

pub async fn create_service_sample_request(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<ServiceSampleRequest>,
) -> Json<serde_json::Value> {
    let user_id = match parse_claim_user_id(&claims) {
        Ok(id) => id,
        Err(message) => return Json(json!({"success": false, "message": message})),
    };

    let brief = payload.brief.trim();
    if brief.is_empty() {
        return Json(json!({"success": false, "message": "A sample brief is required"}));
    }

    let is_unlimited = claims.is_staff || claims.is_superuser;
    let limit = service_sample_free_limit();
    let used = if is_unlimited {
        0
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM service_sample_requests WHERE user_id = $1 AND source = 'videosync_service'",
        )
        .bind(user_id)
        .fetch_one(&state.db_pool)
        .await
        .unwrap_or(0)
    };

    if !is_unlimited && used >= limit {
        return Json(json!({
            "success": false,
            "limit_reached": true,
            "message": "Your included service-page outputs are used up. Upgrade to continue generating more output.",
            "upgrade_url": "/subscribe",
            "used": used,
            "limit": limit,
            "remaining": 0
        }));
    }

    let service_slug = payload.service_slug.trim();
    let reference_url = payload
        .reference_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let prospect_name = payload
        .prospect_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let source = payload
        .source
        .unwrap_or_else(|| "videosync_service".to_string());
    let session_uuid = uuid::Uuid::new_v4().to_string();
    let session_title =
        build_service_sample_chat_title(service_slug, brief, prospect_name.as_deref());

    let session_db_id = match get_or_create_session(&state, &session_uuid, Some(user_id)).await {
        Ok(id) => id,
        Err(error) => {
            return Json(json!({
                "success": false,
                "message": format!("Failed to create chat session for this request: {error}")
            }));
        }
    };

    let _ = sqlx::query(
        "UPDATE chat_sessions
         SET title = $1, updated_at = NOW()
         WHERE id = $2",
    )
    .bind(&session_title)
    .bind(session_db_id)
    .execute(&state.db_pool)
    .await;

    let request_id = uuid::Uuid::new_v4();
    let prompt = build_service_sample_prompt(
        service_slug,
        reference_url.as_deref(),
        prospect_name.as_deref(),
        brief,
    );
    let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
    let workflow_id = match workflow_runtime
        .create_or_reuse_workflow(crate::services::NewWorkflow {
            idempotency_key: Some(format!(
                "service-sample:{user_id}:{session_uuid}:{service_slug}:{request_id}"
            )),
            workflow_type: "service_sample_generation".to_string(),
            status: crate::services::WorkflowStatus::Planning,
            session_uuid: Some(session_uuid.clone()),
            user_id: Some(user_id),
            source_table: Some("service_sample_requests".to_string()),
            source_record_id: Some(request_id),
            request_summary: session_title.clone(),
            current_step: Some("request_received".to_string()),
            metadata: json!({
                "service_slug": service_slug,
                "source": source.clone(),
                "reference_url": reference_url.clone(),
                "prospect_name": prospect_name.clone(),
                "brief": brief,
            }),
            artifact_requirements: json!([
                {
                    "kind": "buyer_facing_sample",
                    "required": true,
                    "must_be_playable": true
                }
            ]),
        })
        .await
    {
        Ok(id) => id,
        Err(error) => {
            return Json(json!({
                "success": false,
                "message": format!("Failed to create durable workflow for this sample request: {error}")
            }));
        }
    };

    let _ = workflow_runtime
        .append_event(
            workflow_id,
            "request_received",
            Some("request_received"),
            "Service sample request captured and ready to be routed into the AI workspace.",
            json!({
                "service_slug": service_slug,
                "request_id": request_id,
            }),
        )
        .await;

    let inserted = sqlx::query(
        "INSERT INTO service_sample_requests
            (id, user_id, service_slug, source, reference_url, prospect_name, brief, generated_prompt, workflow_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(request_id)
    .bind(user_id)
    .bind(service_slug)
    .bind(&source)
    .bind(reference_url.as_deref())
    .bind(prospect_name.as_deref())
    .bind(brief)
    .bind(&prompt)
    .bind(workflow_id)
    .execute(&state.db_pool)
    .await;

    if let Err(error) = inserted {
        let _ = workflow_runtime
            .mark_failed(
                workflow_id,
                Some("request_persistence"),
                &format!("Failed to store sample request: {error}"),
                None,
            )
            .await;
        return Json(json!({
            "success": false,
            "message": format!("Failed to store sample request: {error}")
        }));
    }

    let _ = workflow_runtime
        .heartbeat(
            workflow_id,
            crate::services::WorkflowStatus::Queued,
            Some("awaiting_chat_execution"),
            "The sample request has been stored and is waiting for the AI workspace to start generation.",
            json!({
                "request_id": request_id,
                "chat_source": "service-page",
            }),
        )
        .await;

    let remaining = if is_unlimited {
        limit
    } else {
        std::cmp::max(0, limit - (used + 1))
    };

    let chat_url = format!(
        "/chat/{session_uuid}?{}",
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("prompt", &prompt)
            .append_pair("autosend", "1")
            .append_pair("source", "service-page")
            .append_pair("service", service_slug)
            .append_pair("sample_request_id", &request_id.to_string())
            .append_pair("workflow_id", &workflow_id.to_string())
            .finish()
    );

    Json(json!({
        "success": true,
        "request_id": request_id.to_string(),
        "workflow_id": workflow_id.to_string(),
        "chat_url": chat_url,
        "limit": limit,
        "used": if is_unlimited { 0 } else { used + 1 },
        "remaining": remaining,
        "unlimited": is_unlimited
    }))
}

pub async fn landing_page() -> Html<String> {
    let is_studio = std::env::var("STUDIO_MODE")
        .ok()
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    if is_studio {
        Html(build_studio_landing_page_html())
    } else {
        Html(build_modern_landing_page_html().to_string())
    }
}

pub async fn services_overview_page() -> Html<String> {
    Html(build_services_overview_page_html())
}

pub async fn saas_launch_pack_page() -> Html<String> {
    Html(build_service_offer_page_html(
        "saas-launch-pack",
        "SaaS Demo Video",
        "$399-$1,200+",
        "Send your website or app URL. Our AI agent turns it into a polished product demo in hours — not weeks. One demo or a hundred: volume pricing makes multiple campaigns, hook variations, and localization affordable.",
        "Built for founders, product marketers, agencies, and sales teams that need polished product video at volume — produce 5, 10, or 20 demos for the same cost a traditional studio would charge for one.",
        "The simple offer: send a live product URL, screenshots, app recording, or launch brief and get a buyer-facing demo video package. The full pack includes the main video, 3 hooks/captions, a thumbnail or hero concept, and a delivery page with downloads. Our AI agent produces faster than any human editor — so you can run more launches, A/B test more angles, and produce more content for a fraction of agency cost.",
         "/subscribe",
         "Start 7-day trial",
         "/campaigns/new?service=landing_page",
         "Create campaign",
         &[
             "$399 starter demo: one tightly scoped product demo/promo video, usually 30-90s",
             "$699 launch demo pack: polished demo plus hooks/captions, thumbnail or hero concept, delivery/download page",
             "$1,200+ walkthrough or campaign pack: longer product walkthrough, multiple variants, or stronger motion/voice polish",
            "Optional device, browser, app mockup, narration, and launch scenes",
            "Built for Product Hunt, X, LinkedIn, homepage, onboarding, or sales",
            "Delivered as a review/download link you can share immediately",
        ],
        &[
            "Send the website/app URL and the target buyer.",
            "We decide the strongest story length: pain, product workflow, proof, CTA.",
            "VideoSync builds the video using mockups, motion, narration, captions, and QA where useful.",
            "You receive a delivery page with preview and download links within the rush window.",
        ],
        &[
            ("SaaS founder", "Needs launch-ready product video for a homepage, launch, demo day, or investor update."),
            ("Product marketer", "Needs polished motion assets without hiring a full video team for every release."),
            ("Sales or onboarding team", "Needs clear product video that explains the product faster and shortens the learning curve."),
        ],
        r#"["landing_page","product_mockup","full_stack","scene"]"#,
        r#"["saas-demo-starter","saas-demo-launch","agency-3-videos"]"#,
    ))
}

pub async fn clipper_enhancement_pack_page() -> Html<String> {
    Html(build_service_offer_page_html(
        "clipper-enhancement-pack",
        "Thumbnail & Motion Graphics Pack",
        "$250-$1,200+",
        "High-converting thumbnails, title cards, lower thirds, mockups, and motion assets — produced in hours, not days. Generate 10 thumbnail variants or a full campaign's worth of visuals without hiring a designer.",
        "Built for creators, marketers, agencies, and small teams that need premium visual packaging at volume — more thumbnails, more variants, more A/B tests, all for a fraction of what a designer would charge per asset.",
        "This offer focuses on the platform's strongest visual add-ons: rendered thumbnails, title cards, lower thirds, device mockups, data visuals, and branded motion scenes. The AI agent handles production end-to-end — so you can order 3 variants or 30 and still get them back in the same timeframe.",
        "/manual-clipping",
        "Open video tools",
         "/campaigns/new?service=clipping",
         "Create campaign",
        &[
            "YouTube and social thumbnail variants",
            "Title cards, lower thirds, and branded overlays",
            "Device mockups and promo motion loops",
            "Data visuals, explainer scenes, and support graphics",
            "Polish assets you can reuse across campaigns",
        ],
        &[
            "Share the video, channel, campaign, or design direction you want to improve.",
            "We identify the supporting visuals that will make the content look more premium and click-worthy.",
            "VideoSync produces the thumbnail, motion graphics, mockups, or support scenes around that brief.",
            "You receive ready-to-use assets that fit your channel, launch, or client package.",
        ],
        &[
            ("Creator or YouTube operator", "Needs stronger thumbnails and packaging to improve clicks and presentation."),
            ("Launch marketer", "Needs motion assets that make a release look more polished across channels."),
            ("Agency or freelance editor", "Needs premium visual add-ons without building every graphic from scratch."),
        ],
        r#"["animations","thumbnails","scene","ui_mockup"]"#,
        r#"["clip-enhancement-standard"]"#,
    ))
}

pub async fn thumbnail_hero_pack_page() -> Html<String> {
    Html(build_service_offer_page_html(
        "thumbnail-hero-pack",
        "Thumbnail & Hero Visual Pack",
        "$75-$300+",
        "Click-focused thumbnails, hero visuals, and campaign graphics — delivered same day. Run multiple designs per video, A/B test thumbnails, and never wait on a designer again.",
        "Built for YouTubers, SaaS founders, course sellers, agencies, and operators who need stronger first impressions at scale — more thumbnails, more variants, more campaigns for less.",
        "VideoSync turns a product, video, or campaign brief into thumbnail variants, hero visuals, ad stills, and reusable visual directions — with Gemini multimodal QA before delivery. The AI agent handles volume effortlessly: order 3 thumbnails or 30, the turnaround stays the same.",
         "/campaigns/new?service=thumbnails",
         "Create campaign",
         "/chat",
         "Try one-off in chat",
        &[
            "YouTube thumbnail variants",
            "SaaS/product hero visuals",
            "Ad stills and campaign graphics",
            "Visual direction notes and hooks",
            "Download-ready delivery links",
        ],
        &[
            "Share the product, video, campaign, or audience you want to attract.",
            "The agent creates several visual angles and hook concepts.",
            "VideoSync generates and reviews the strongest thumbnail/hero candidates.",
            "You receive downloadable assets and captions/hooks to test.",
        ],
        &[
            ("YouTube creator", "Needs better click-through without waiting on a designer."),
            ("SaaS founder", "Needs stronger launch and landing-page visuals."),
            ("Agency operator", "Needs fast visual variants for client campaigns."),
        ],
        r#"["thumbnails","generated_images","landing_page","business_explainer"]"#,
        r#"["clip-enhancement-standard"]"#,
    ))
}

pub async fn product_mockup_pack_page() -> Html<String> {
    Html(build_service_offer_page_html(
        "product-mockup-pack",
        "Product Mockup Video Pack",
        "$299-$900+",
        "Send a website, screenshots, or app flow. Our AI agent turns it into animated UI mockups and short product videos — in hours, not weeks. Produce demo videos for every feature, every use case, every update.",
        "Built for SaaS founders, indie hackers, app owners, no-code builders, and agencies selling productized demos at scale — more feature videos, more product angles, more campaigns for the same budget.",
        "This is the visual upgrade for apps that look useful but do not yet feel premium. Send a URL, screenshots, Figma exports, or a written workflow; VideoSync turns it into browser/device scenes, motion callouts, and short product videos. The automated pipeline means you can produce a mockup for every major feature without the cost of a traditional video team.",
         "/campaigns/new?service=product_mockup",
         "Create campaign",
         "/chat",
         "Try one-off in chat",
        &[
            "Browser/device mockup scenes",
            "Short product walkthrough videos",
            "App promo clips and ads",
            "Landing-page hero concepts",
            "Delivery page with downloads",
        ],
        &[
            "Share the product URL, screenshots, or app flow.",
            "The agent identifies the clearest product story and buyer use case.",
            "VideoSync renders UI mockups, narration, motion, and support footage.",
            "You get a shareable delivery link and downloadable media.",
        ],
        &[
            ("Indie hacker", "Needs a product video before paid ads or launch."),
            ("No-code builder", "Needs a polished demo from screenshots and a short brief."),
            ("Agency", "Needs repeatable client mockup videos."),
        ],
        r#"["product_mockup","landing_page","animations","full_stack"]"#,
        r#"["product-mockup-standard"]"#,
    ))
}

pub async fn education_explainer_pack_page() -> Html<String> {
    Html(build_service_offer_page_html(
        "education-explainer-pack",
        "Education Explainer Pack",
        "$300-$1,500+",
        "Animated math/science visuals, diagrams, narration, and long-form explainers — automated end-to-end. Produce a full course curriculum's worth of lessons for the price an agency would charge for one video.",
        "Built for educators, course creators, technical YouTubers, founders, and B2B teams that need concepts explained visually at volume — more lessons, more modules, more content for less.",
        "VideoSync combines animated math/science visuals, diagrams, stock footage, narration, and long-form assembly into lessons, explainer videos, and course modules. The AI agent produces each segment independently — so a 20-video course ships just as fast as a single explainer. No studio, no crew, no markup per video.",
         "/campaigns/new?service=education",
         "Create campaign",
         "/chat",
         "Try one-off in chat",
        &[
            "Animated explainer scenes",
            "Narrated explainers and tutorials",
            "Course lesson videos",
            "Diagrams, formulas, and visual proofs",
            "Long-form assembly with checkpoints",
        ],
        &[
            "Share the concept, lesson outline, or technical topic.",
            "The agent chooses diagrams, formulas, narration, and visual pacing.",
            "VideoSync renders recoverable segments and reviews the outputs.",
            "You receive a complete video plus reusable assets.",
        ],
        &[
            ("Course creator", "Needs lesson videos without manually animating every concept."),
            ("Technical founder", "Needs a clear product or API explainer."),
            ("YouTube educator", "Needs repeatable educational video production."),
        ],
        r#"["education","manim","latex","long_form"]"#,
        r#"["education-explainer-standard"]"#,
    ))
}

pub async fn blender_scene_pack_page() -> Html<String> {
    Html(build_service_offer_page_html(
        "blender-scene-pack",
        "3D/2D Animation Scene Pack",
        "$500-$2,500+",
        "Animated 3D/2D scenes, product animations, 3D explainers, and cinematic visuals — generated by AI agents in hours. Render multiple product angles and animation variants for the cost of a single studio shoot.",
        "Built for product teams, creators, agencies, educators, and technical brands that need 3D visuals at volume — more scenes, more angles, more variations for the same budget.",
        "VideoSync generates animated 3D/2D scenes alongside editing, narration, QA, thumbnails, and delivery pages to produce stronger demos and explainers. The automated pipeline lets you order multiple product angles, animation styles, or scene variants without per-scene overhead of a traditional studio.",
         "/campaigns/new?service=full_stack",
         "Create campaign",
         "/chat",
         "Try one-off in chat",
        &[
            "2D/3D product scenes",
            "Animated models and explainers",
            "Title cards and lower thirds",
            "Data visuals and cinematic loops",
            "QA-reviewed rendered assets",
        ],
        &[
            "Describe the object, scene, or animation goal.",
            "The agent orchestrates editing, animated scenes, voiceovers, and image generation as needed.",
            "Rendered assets are reviewed and packaged with downloads.",
            "Scenes can be inserted into longer product or education videos.",
        ],
        &[
            ("Product marketer", "Needs visuals that make the product feel premium."),
            ("Educator", "Needs physical or abstract concepts animated clearly."),
            ("Agency", "Needs unique visuals clients cannot get from template editors."),
        ],
        r#"["blender","3d_scene","animations","full_stack"]"#,
        r#"["blender-scene-standard"]"#,
    ))
}

pub async fn voice_audio_pack_page() -> Html<String> {
    Html(build_service_offer_page_html(
        "voice-audio-pack",
        "Voice & Audio Production Pack",
        "$99-$750+",
        "Narration, podcast-style audio, voiceovers, summaries, and audio-backed video packages — produced same day. Generate 5, 10, or 50 voiceovers without a recording studio or voice talent budget.",
        "Built for founders, creators, educators, agencies, and newsletter operators who need narration or audio content at scale — more scripts, more variations, more formats for less.",
        "VideoSync generates scripts, voiceovers, narrated summaries, audio visualizers, and video packages that combine narration with motion assets. The AI agent handles script writing, voice generation, and assembly — so producing a batch of 20 narrated clips costs roughly the same as producing one.",
         "/campaigns/new?service=voice_audio",
         "Create campaign",
         "/chat",
         "Try one-off in chat",
        &[
            "Voiceover and narration scripts",
            "AI-assisted voice and narration outputs",
            "Podcast/video summaries",
            "Audio visualizers",
            "Narrated videos and delivery links",
        ],
        &[
            "Share the source, topic, or script direction.",
            "The agent writes or adapts narration for the goal.",
            "VideoSync generates audio and optionally pairs it with visuals.",
            "You receive downloadable audio/video assets.",
        ],
        &[
            ("Founder", "Needs a clear narrated demo or update."),
            ("Creator", "Needs voiceover-backed clips and summaries."),
            ("Agency", "Needs fast narration for client deliverables."),
        ],
        r#"["voice_audio","summary","long_form"]"#,
        r#"["audio-standard"]"#,
    ))
}

pub async fn mixed_agency_bundle_page() -> Html<String> {
    Html(build_service_offer_page_html(
        "mixed-agency-bundle",
        "Agency Client Pack (3 Videos)",
        "$1,500 for 3 client videos",
        "For Webflow, Framer, SaaS, and marketing agencies: send 3 client websites and get 3 client-ready demo videos in hours. Scale from 3 to 30 clients without hiring a video team.",
        "Built for agencies, freelancers, Webflow/Framer studios, no-code builders, and SaaS marketers who already have clients but need faster, higher-volume video fulfillment — fulfill more clients, produce more assets, charge less, keep more margin.",
        "The plain offer: agencies send client websites or app URLs, and VideoSync produces demo/promo videos plus supporting assets they can deliver under their own brand. The automated pipeline means you can take on more clients without scaling your team — 3 videos or 30, the turnaround stays the same. No studio overhead, no per-video markup, just white-label video fulfillment at agency-friendly pricing.",
         "/campaigns/new?service=full_stack",
         "Create campaign",
         "/chat",
         "Try one-off in chat",
        &[
            "$1,500 pack: 3 client website/app demo videos",
            "Each client gets a delivery page with preview and downloads",
            "Optional hook/caption variants, thumbnails, mockups, and narration",
            "Designed so agencies can resell the assets to their own clients",
            "Upsell path into monthly fulfillment once one pack works",
        ],
        &[
            "Agency sends 3 client URLs and the goal for each video.",
            "We turn each site/app into a clear demo/promo concept, usually short-form first with longer walkthroughs available.",
            "VideoSync generates, reviews, and packages each client video.",
            "Agency receives 3 delivery/download links they can send or resell.",
        ],
        &[
            ("Webflow or Framer agency", "Already builds sites and can resell demo videos as a launch add-on."),
            ("SaaS marketing agency", "Needs product videos for clients without hiring a video team."),
            ("No-code builder or consultant", "Can offer website-to-video as an upsell after shipping an app or landing page."),
        ],
        r#"["bundle","full_stack","long_form","thumbnails","voice_audio","blender","education"]"#,
        r#"["agency-3-videos"]"#,
    ))
}

pub async fn creator_manager_fulfillment_page() -> Html<String> {
    Html(build_service_offer_page_html(
        "creator-manager-fulfillment",
        "Agency Production Backend",
        "$999-$3,000+/month",
        "A private production backend — produce 10x more client deliverables without hiring editors. VideoSync handles the output; you keep the client and the brand.",
        "Built for boutique agencies, creator managers, consultants, and operators who sell video services and need reliable, scalable fulfillment without the cost of an in-house production team.",
        "VideoSync works best here as a backend, not a personality. You keep the client relationship and use the platform to produce demos, thumbnails, motion graphics, narrated explainers, delivery pages, and repeatable monthly output under your own brand. The AI agent pipeline means you can fulfill 30 client videos a month with the same effort it used to take to produce 3 — your margin scales with volume, not headcount.",
        "/dashboard",
        "Open the workspace",
        "/api-access",
        "View API access",
        &[
            "White-label production support across multiple client accounts",
            "Product demos, thumbnails, motion graphics, and narrated explainers",
            "Delivery links that make review and handoff easier",
            "Repeatable monthly fulfillment instead of one-off scrambling",
            "A backend that can grow from manual work into API-driven workflows",
        ],
        &[
            "You sell the offer under your own brand and keep the client relationship.",
            "We help turn the brief into the right production workflow and output mix.",
            "VideoSync fulfills the deliverables behind the scenes with the same production stack used across the platform.",
            "Fulfill with VideoSync’s editing, generation, and delivery stack.",
        ],
        &[
            ("Boutique agency owner", "Needs more delivery capacity without turning every new client into an operations problem."),
            ("Creator manager", "Needs a production backend that helps keep fulfillment consistent across a small roster."),
            ("Solo operator", "Needs a way to sell a larger service without hiring a full in-house team first."),
        ],
        r#"["full_stack","thumbnails","scene","landing_page"]"#,
        r#"[]"#,
    ))
}

pub async fn x402_asset_api_page() -> Html<String> {
    Html(build_x402_docs_page_html())
}

pub async fn kick_com_clipping_page() -> Html<String> {
    Html(build_service_offer_page_html(
        "kick-com-clipping",
        "Kick.com Clipping",
        "$75-$300+",
        "Paste a Kick.com VOD link. Our AI agent downloads the stream, extracts your requested highlights, and delivers ready-to-post clips with captions and thumbnails — in hours, not days.",
        "Built for Kick streamers, editors, clip channels, social media managers, and anyone who needs professional highlight clips from Kick VODs without screen recording or manual editing.",
        "The simple offer: send a Kick.com VOD URL and describe the moments you want. The full pack includes 3 extracted clips, each with search-optimized captions, a hook title card, a click-focused thumbnail, and a delivery page with download links. Supports gaming highlights, IRL moments, donation reactions, and any other VOD content.",
        "/manual-clipping",
        "Open clipping tools",
         "/campaigns/new?service=kick_auto_clipper",
         "Create campaign",
        &[
            "Clip packs from public Kick.com VODs — no screen recording needed",
            "Easy: send a VOD URL and tell us what moments to extract",
            "Captions, hook title cards, and thumbnails included per clip",
            "Vertical and horizontal format support (Shorts, TikTok, Twitter, YouTube)",
            "Auto-publish across YouTube, TikTok, Instagram, and X via Zernio integration",
            "Campaign Engine: schedule daily clip generation + cross-platform posting",
            "Delivered as a review/download link you can share immediately",
        ],
        &[
            "Send the Kick.com VOD URL and describe the highlights you want extracted.",
            "We download the VOD and identify the requested moments.",
            "VideoSync clips each moment, adds captions, hook text, and thumbnail.",
            "Clips are auto-published to your connected social accounts via Zernio.",
            "You receive a delivery page with preview and download links.",
        ],
        &[
            ("Kick streamer or editor", "Needs professional highlight clips from Kick VODs without uploading to third-party tools or screen recording."),
            ("Clip channel operator", "Needs consistent, captioned clips from multiple Kick streamers for a compilation or clipping channel with daily posting."),
            ("Social media manager", "Needs platform-ready short-form clips from Kick content for Twitter, TikTok, or Instagram cross-posting via automated campaigns."),
        ],
        r#"["clips","captions","thumbnails","scene"]"#,
        r#"["standard"]"#,
    ))
}

// ── Campaign Dashboard Pages ─────────────────────────────────────────────────

pub async fn campaigns_list_page(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Html<String> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let rows = sqlx::query_as::<_, (uuid::Uuid, String, String, String, i32, i32, i32, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, name, service_type, status, posts_per_day, \
                total_posts_planned, total_posts_published, created_at \
         FROM campaigns WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();

    let campaigns_html: String = if rows.is_empty() {
        r#"<div class="empty-state"><h2>No campaigns yet</h2><p>Create your first campaign to start generating and posting content automatically.</p><a class="btn btn-primary" href="/campaigns/new">Create Campaign</a></div>"#.to_string()
    } else {
        rows.iter().map(|(id, name, service_type, status, per_day, planned, published, created)| {
            let status_badge = match status.as_str() {
                "active" => r#"<span class="badge badge-active">Active</span>"#,
                "paused" => r#"<span class="badge badge-paused">Paused</span>"#,
                "completed" => r#"<span class="badge badge-completed">Completed</span>"#,
                "cancelled" => r#"<span class="badge badge-cancelled">Cancelled</span>"#,
                _ => r#"<span class="badge">Pending</span>"#,
            };
            let service_label = format_service_type(service_type);
            format!(
                r#"<a class="campaign-card" href="/campaigns/{id}">
                    <div class="card-top">
                        <div class="service-tag">{service_label}</div>
                        {status_badge}
                    </div>
                    <h3>{name}</h3>
                    <div class="card-meta">
                        <span>{published}/{planned} posts</span>
                        <span>{per_day}/day</span>
                        <span>{created}</span>
                    </div>
                </a>"#
            )
        }).collect()
    };

    Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>My Campaigns — VideoSync</title>
<style>
    :root {{ --bg:#07111d; --panel:rgba(9,18,31,0.84); --line:rgba(148,163,184,0.16); --text:#e5eefb; --muted:#a8b8d3; --blue:#3b82f6; --green:#22c55e; }}
    * {{ box-sizing:border-box; }}
    body {{ margin:0; font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; background:var(--bg); color:var(--text); }}
    .shell {{ max-width:960px; margin:0 auto; padding:32px 20px 72px; }}
    .topbar {{ display:flex; justify-content:space-between; align-items:center; margin-bottom:28px; flex-wrap:wrap; gap:1rem; }}
    .brand {{ color:#fff; text-decoration:none; font-weight:800; font-size:1.3rem; }}
    .nav-links a {{ color:#c7d8f6; text-decoration:none; padding:0.65rem 1rem; border:1px solid rgba(148,163,184,0.2); border-radius:999px; background:rgba(8,15,28,0.75); margin-left:0.5rem; }}
    h1 {{ margin:0 0 1.5rem; font-size:1.8rem; }}
    .campaigns-grid {{ display:grid; grid-template-columns:repeat(auto-fill,minmax(280px,1fr)); gap:1rem; }}
    .campaign-card {{ display:block; padding:1.25rem; border-radius:16px; background:var(--panel); border:1px solid var(--line); text-decoration:none; color:var(--text); transition:border-color 0.2s; }}
    .campaign-card:hover {{ border-color:var(--blue); }}
    .card-top {{ display:flex; justify-content:space-between; align-items:center; margin-bottom:0.8rem; }}
    .service-tag {{ font-size:0.78rem; padding:0.25rem 0.6rem; border-radius:999px; background:rgba(59,130,246,0.15); color:#93c5fd; }}
    .badge {{ font-size:0.72rem; padding:0.2rem 0.5rem; border-radius:999px; }}
    .badge-active {{ background:rgba(34,197,94,0.15); color:#86efac; }}
    .badge-paused {{ background:rgba(251,191,36,0.15); color:#fde68a; }}
    .badge-completed {{ background:rgba(99,102,241,0.15); color:#c4b5fd; }}
    .badge-cancelled {{ background:rgba(239,68,68,0.15); color:#fca5a5; }}
    .campaign-card h3 {{ margin:0 0 0.5rem; font-size:1.05rem; }}
    .card-meta {{ display:flex; gap:0.8rem; font-size:0.82rem; color:var(--muted); }}
    .empty-state {{ text-align:center; padding:4rem 1rem; }}
    .empty-state h2 {{ margin:0 0 0.5rem; }}
    .empty-state p {{ color:var(--muted); }}
    .btn {{ display:inline-flex; align-items:center; padding:0.7rem 1.2rem; border-radius:5px; text-decoration:none; font-weight:700; cursor:pointer; }}
    .btn-primary {{ background:linear-gradient(135deg,var(--blue),#2563eb); color:#fff; border:0; }}
</style>
</head>
<body>
<div class="shell">
    <div class="topbar">
        <a class="brand" href="/">VideoSync</a>
        <div class="nav-links">
            <a href="/campaigns/new">+ New Campaign</a>
            <a href="/chat">Chat</a>
            <a href="/services">Services</a>
        </div>
    </div>
    <h1>My Campaigns</h1>
    <div class="campaigns-grid">{campaigns_html}</div>
</div>
</body>
</html>"#
    ))
}

pub async fn campaigns_new_page(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Html<String> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);

    // Check subscription — non-subscribers get a paywall
    let sub_ok = crate::handlers::chat::subscription_ok(&state, user_id).await.unwrap_or(false);
    if !sub_ok {
        return Html(format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Subscribe — VideoSync</title>
<style>
    :root {{ --bg:#07111d; --panel:rgba(9,18,31,0.84); --text:#e5eefb; --blue:#3b82f6; }}
    * {{ box-sizing:border-box; }}
    body {{ margin:0; font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; background:var(--bg); color:var(--text); display:flex; align-items:center; justify-content:center; min-height:100vh; }}
    .box {{ text-align:center; max-width:480px; padding:2rem; }}
    h1 {{ font-size:2rem; margin:0 0 1rem; }}
    p {{ color:#a8b8d3; line-height:1.6; }}
    .btn {{ display:inline-block; padding:0.8rem 1.5rem; border-radius:5px; background:linear-gradient(135deg,var(--blue),#2563eb); color:#fff; text-decoration:none; font-weight:700; margin-top:1rem; }}
</style>
</head>
<body>
<div class="box">
    <h1>Subscription Required</h1>
    <p>Campaigns are part of our managed production service. Subscribe to get access to the campaign dashboard, daily content generation, and auto-publishing via Zernio.</p>
    <a class="btn" href="/subscribe">Subscribe — $15/mo</a>
    <p style="margin-top:1rem;font-size:0.85rem;">Already subscribed? <a href="/login" style="color:#93c5fd;">Log in</a></p>
</div>
</body>
</html>"#
        ));
    }

    let preselect_service = query.get("service").cloned().unwrap_or_default();
    let service_options = [
        ("landing_page", "SaaS Demo Video"),
        ("product_mockup", "Product Mockup"),
        ("education", "Education Explainer"),
        ("clipping", "Clip Enhancement"),
        ("kick_auto_clipper", "Kick.com Clipping"),
        ("business_explainer", "Business Explainer"),
        ("manim_explainer", "Manim Explainer"),
        ("voice_audio", "Voice & Audio"),
        ("full_stack", "Full Stack Agency"),
        ("whiteboard_animation", "Whiteboard Animation"),
        ("kinetic_typography", "Kinetic Typography"),
        ("animated_infographic", "Animated Infographic"),
        ("algorithm_viz", "Algorithm Visualization"),
        ("investor_pitch", "Investor Pitch"),
        ("year_in_review", "Year in Review"),
        ("isometric_explainer", "Isometric Explainer"),
    ];

    let options_html: String = service_options.iter().map(|(val, label)| {
        let sel = if *val == preselect_service { " selected" } else { "" };
        format!(r#"<option value="{val}"{sel}>{label}</option>"#)
    }).collect();

    Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>New Campaign — VideoSync</title>
<style>
    :root {{ --bg:#07111d; --panel:rgba(9,18,31,0.84); --line:rgba(148,163,184,0.16); --text:#e5eefb; --muted:#a8b8d3; --blue:#3b82f6; --green:#22c55e; }}
    * {{ box-sizing:border-box; }}
    body {{ margin:0; font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; background:var(--bg); color:var(--text); }}
    .shell {{ max-width:640px; margin:0 auto; padding:32px 20px 72px; }}
    h1 {{ margin:0 0 1.5rem; font-size:1.6rem; }}
    .topbar {{ display:flex; justify-content:space-between; align-items:center; margin-bottom:28px; }}
    .brand {{ color:#fff; text-decoration:none; font-weight:800; font-size:1.3rem; }}
    .nav-links a {{ color:#c7d8f6; text-decoration:none; padding:0.6rem 0.9rem; border:1px solid rgba(148,163,184,0.2); border-radius:999px; background:rgba(8,15,28,0.75); margin-left:0.4rem; font-size:0.88rem; }}
    form {{ display:flex; flex-direction:column; gap:1.2rem; }}
    label {{ font-weight:600; font-size:0.9rem; color:#dbeafe; }}
    input, select, textarea {{ width:100%; padding:0.7rem; border-radius:8px; border:1px solid var(--line); background:rgba(15,23,42,0.7); color:#fff; font-size:0.95rem; }}
    input:focus, select:focus, textarea:focus {{ outline:none; border-color:var(--blue); }}
    textarea {{ min-height:100px; resize:vertical; }}
    .form-row {{ display:grid; grid-template-columns:1fr 1fr; gap:1rem; }}
    .btn {{ padding:0.8rem 1.2rem; border-radius:5px; font-weight:700; cursor:pointer; border:0; font-size:1rem; }}
    .btn-primary {{ background:linear-gradient(135deg,var(--blue),#2563eb); color:#fff; }}
    .btn-secondary {{ background:rgba(15,23,42,0.7); border:1px solid var(--line); color:#dbeafe; text-decoration:none; text-align:center; }}
    .hint {{ color:var(--muted); font-size:0.82rem; margin-top:0.3rem; }}
    .schedule-entry {{ display:flex; gap:0.5rem; align-items:center; }}
    .schedule-entry input {{ width:auto; flex:1; }}
    .schedule-entry select {{ width:auto; flex:1; }}
    .add-btn {{ background:rgba(59,130,246,0.15); color:#93c5fd; border:1px dashed rgba(59,130,246,0.3); padding:0.5rem; border-radius:8px; cursor:pointer; font-size:0.85rem; }}
</style>
</head>
<body>
<div class="shell">
    <div class="topbar">
        <a class="brand" href="/">VideoSync</a>
        <div class="nav-links">
            <a href="/campaigns">My Campaigns</a>
            <a href="/chat">Chat</a>
        </div>
    </div>
    <h1>Create Campaign</h1>
    <form id="campaignForm">
        <div>
            <label>Campaign Name</label>
            <input type="text" name="name" required placeholder="e.g., Weekly Education Series">
        </div>
        <div>
            <label>Service Type</label>
            <select name="service_type" id="serviceType">{options_html}</select>
        </div>
        <div>
            <label>Brief / Topic</label>
            <textarea name="brief" required placeholder="Describe the content you want generated daily. For example: '3-minute educational explainers about calculus concepts'"></textarea>
        </div>
        <div class="form-row">
            <div>
                <label>Style</label>
                <select name="style">
                    <option value="cinematic">Cinematic</option>
                    <option value="modern">Modern</option>
                    <option value="minimal">Minimal</option>
                    <option value="educational">Educational</option>
                </select>
            </div>
            <div>
                <label>Duration (seconds)</label>
                <input type="number" name="duration" value="30" min="5" max="600">
            </div>
        </div>
        <div class="form-row">
            <div>
                <label>Posts Per Day</label>
                <input type="number" name="posts_per_day" value="3" min="1" max="10">
            </div>
            <div>
                <label>Start Date</label>
                <input type="date" name="start_date" id="startDate">
            </div>
        </div>
        <div>
            <label>End Date</label>
            <input type="date" name="end_date" id="endDate">
        </div>
        <div>
            <label>Schedule Times</label>
            <p class="hint">Set the times each day when posts should be published.</p>
            <div id="scheduleEntries">
                <div class="schedule-entry">
                    <input type="time" name="schedule_time" value="08:00">
                    <select name="schedule_platform">
                        <option value="youtube">YouTube</option>
                        <option value="tiktok">TikTok</option>
                        <option value="instagram">Instagram</option>
                        <option value="twitter">X / Twitter</option>
                    </select>
                </div>
                <div class="schedule-entry">
                    <input type="time" name="schedule_time" value="12:00">
                    <select name="schedule_platform">
                        <option value="tiktok">TikTok</option>
                        <option value="youtube">YouTube</option>
                        <option value="instagram">Instagram</option>
                        <option value="twitter">X / Twitter</option>
                    </select>
                </div>
                <div class="schedule-entry">
                    <input type="time" name="schedule_time" value="17:00">
                    <select name="schedule_platform">
                        <option value="instagram">Instagram</option>
                        <option value="youtube">YouTube</option>
                        <option value="tiktok">TikTok</option>
                        <option value="twitter">X / Twitter</option>
                    </select>
                </div>
            </div>
            <button type="button" class="add-btn" onclick="addScheduleEntry()">+ Add Time Slot</button>
        </div>
        <div>
            <label>Social Accounts (Zernio)</label>
            <p class="hint">Connect your social accounts via <a href="/admin/zernio" style="color:#93c5fd;">Zernio settings</a> first, then select which profiles to post to.</p>
            <select name="zernio_profile_id" id="zernioProfile">
                <option value="">Select a Zernio profile...</option>
            </select>
        </div>
        <div>
            <label>Connected Platforms</label>
            <div id="platformAccounts" style="display:flex;flex-direction:column;gap:0.5rem;">
                <p class="hint">Connect accounts in Zernio settings first, then they'll appear here.</p>
            </div>
        </div>
        <button type="submit" class="btn btn-primary">Create Campaign</button>
    </form>
</div>
<script>
function addScheduleEntry() {{
    const div = document.createElement('div');
    div.className = 'schedule-entry';
    div.innerHTML = '<input type="time" name="schedule_time" value="09:00"><select name="schedule_platform"><option value="youtube">YouTube</option><option value="tiktok">TikTok</option><option value="instagram">Instagram</option><option value="twitter">X / Twitter</option></select><button type="button" onclick="this.parentElement.remove()" style="background:none;border:none;color:#ef4444;cursor:pointer;font-size:1.2rem;">×</button>';
    document.getElementById('scheduleEntries').appendChild(div);
}}

// Set default dates
document.getElementById('startDate').value = new Date().toISOString().split('T')[0];
const endDate = new Date(); endDate.setMonth(endDate.getMonth() + 1);
document.getElementById('endDate').value = endDate.toISOString().split('T')[0];

// Load Zernio profile for platform accounts
async function loadZernioAccounts() {{
    try {{
        const resp = await fetch('/api/admin/zernio/status');
        const data = await resp.json();
        if (data.success && data.profiles) {{
            const sel = document.getElementById('zernioProfile');
            data.profiles.forEach(p => {{
                const opt = document.createElement('option');
                opt.value = p.id;
                opt.textContent = p.name;
                sel.appendChild(opt);
            }});
        }}
        if (data.success && data.accounts) {{
            const container = document.getElementById('platformAccounts');
            container.innerHTML = '';
            data.accounts.forEach(a => {{
                if (a.connected) {{
                    const label = document.createElement('label');
                    label.style.cssText = 'display:flex;align-items:center;gap:0.5rem;font-weight:400;';
                    label.innerHTML = '<input type="checkbox" name="platform_account" value=\'' + JSON.stringify({{platform: a.platform, accountId: a.id}}) + '\'> ' + a.platform + ' (@' + (a.username || 'connected') + ')';
                    container.appendChild(label);
                }}
            }});
        }}
    }} catch(e) {{ console.log('Zernio load skipped'); }}
}}
loadZernioAccounts();

document.getElementById('campaignForm').addEventListener('submit', async (e) => {{
    e.preventDefault();
    const fd = new FormData(e.target);
    const schedule = [];
    document.querySelectorAll('#scheduleEntries .schedule-entry').forEach(entry => {{
        const time = entry.querySelector('input[type="time"]').value;
        const platform = entry.querySelector('select').value;
        schedule.push({{time, platform}});
    }});
    const platforms = [];
    document.querySelectorAll('input[name="platform_account"]:checked').forEach(cb => {{
        platforms.push(JSON.parse(cb.value));
    }});
    const payload = {{
        name: fd.get('name'),
        service_type: fd.get('service_type'),
        brief: fd.get('brief'),
        style: fd.get('style'),
        duration: parseFloat(fd.get('duration')),
        posts_per_day: parseInt(fd.get('posts_per_day')),
        start_date: new Date(fd.get('start_date')).toISOString(),
        end_date: new Date(fd.get('end_date')).toISOString(),
        schedule: schedule,
        platforms: platforms,
        zernio_profile_id: fd.get('zernio_profile_id') || undefined,
    }};
    try {{
        const resp = await fetch('/api/campaigns', {{ method: 'POST', headers: {{'Content-Type':'application/json'}}, body: JSON.stringify(payload) }});
        const data = await resp.json();
        if (data.success) {{
            window.location.href = '/campaigns/' + data.id;
        }} else {{
            alert('Error: ' + (data.error || 'Unknown'));
        }}
    }} catch(e) {{ alert('Network error: ' + e); }}
}});
</script>
</body>
</html>"#
    ))
}

pub async fn campaigns_detail_page(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<uuid::Uuid>,
) -> Html<String> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);

    let campaign = sqlx::query_as::<_, (uuid::Uuid, String, String, String, String, f64, serde_json::Value, serde_json::Value, i32, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>, Option<String>, String, i32, i32)>(
        "SELECT id, name, service_type, brief, style, duration, schedule, platforms, \
                posts_per_day, start_date, end_date, zernio_profile_id, status, \
                total_posts_planned, total_posts_published \
         FROM campaigns WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await;

    let (id, name, service_type, brief, style, duration, _schedule, _platforms, posts_per_day,
         start_date, end_date, _zernio_profile_id, status, total_planned, total_published) = match campaign {
        Ok(Some(r)) => r,
        Ok(None) => return Html(r#"<html><body><h1>Campaign not found</h1><a href="/campaigns">Back</a></body></html>"#.to_string()),
        Err(_) => return Html(r#"<html><body><h1>Error loading campaign</h1><a href="/campaigns">Back</a></body></html>"#.to_string()),
    };

    let posts = sqlx::query_as::<_, (uuid::Uuid, i32, i32, chrono::DateTime<chrono::Utc>, Option<String>, Option<String>, String, Option<String>)>(
        "SELECT id, day_number, slot_index, scheduled_at, media_r2_url, caption, status, zernio_post_id \
         FROM campaign_posts WHERE campaign_id = $1 ORDER BY day_number, slot_index LIMIT 100",
    )
    .bind(id)
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();

    let posts_html: String = posts.iter().map(|(_pid, day, slot, scheduled_at, media_url, caption, post_status, zernio_id)| {
        let status_icon = match post_status.as_str() {
            "pending_generation" => "⏳",
            "rendering" => "🔄",
            "scheduled" => "📅",
            "published" => "✅",
            "failed" => "❌",
            _ => "⬜",
        };
        let media_link = match media_url {
            Some(url) => format!(r#"<a href="{url}" target="_blank" style="color:#93c5fd;">View</a>"#),
            None => "—".to_string(),
        };
        let zernio_link = match zernio_id {
            Some(zid) => format!(" ({})", zid.chars().take(8).collect::<String>()),
            None => String::new(),
        };
        format!(
            r#"<tr>
                <td>Day {day}</td>
                <td>Slot {slot}</td>
                <td>{scheduled_at}</td>
                <td>{status_icon} {post_status}{zernio_link}</td>
                <td>{media_link}</td>
                <td>{caption}</td>
            </tr>"#
        )
    }).collect();

    let status_badge = match status.as_str() {
        "active" => r#"<span class="badge badge-active">Active</span>"#,
        "paused" => r#"<span class="badge badge-paused">Paused</span>"#,
        "completed" => r#"<span class="badge badge-completed">Completed</span>"#,
        "cancelled" => r#"<span class="badge badge-cancelled">Cancelled</span>"#,
        _ => r#"<span class="badge">Pending</span>"#,
    };

    let status_actions = match status.as_str() {
        "active" => r#"<button class="btn btn-warning" onclick="updateStatus('paused')">Pause</button><button class="btn btn-danger" onclick="updateStatus('cancelled')">Cancel</button>"#,
        "paused" => r#"<button class="btn btn-success" onclick="updateStatus('active')">Resume</button><button class="btn btn-danger" onclick="updateStatus('cancelled')">Cancel</button>"#,
        _ => String::new(),
    };

    let start_str = start_date.format("%b %d, %Y").to_string();
    let end_str = end_date.format("%b %d, %Y").to_string();
    let service_label = format_service_type(&service_type);

    Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{name} — VideoSync</title>
<style>
    :root {{ --bg:#07111d; --panel:rgba(9,18,31,0.84); --line:rgba(148,163,184,0.16); --text:#e5eefb; --muted:#a8b8d3; --blue:#3b82f6; --green:#22c55e; --red:#ef4444; --amber:#f59e0b; }}
    * {{ box-sizing:border-box; }}
    body {{ margin:0; font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; background:var(--bg); color:var(--text); }}
    .shell {{ max-width:960px; margin:0 auto; padding:32px 20px 72px; }}
    .topbar {{ display:flex; justify-content:space-between; align-items:center; margin-bottom:28px; flex-wrap:wrap; gap:1rem; }}
    .brand {{ color:#fff; text-decoration:none; font-weight:800; font-size:1.3rem; }}
    .nav-links a {{ color:#c7d8f6; text-decoration:none; padding:0.65rem 1rem; border:1px solid rgba(148,163,184,0.2); border-radius:999px; background:rgba(8,15,28,0.75); margin-left:0.5rem; }}
    .campaign-header {{ display:flex; justify-content:space-between; align-items:flex-start; flex-wrap:wrap; gap:1rem; margin-bottom:1.5rem; }}
    .campaign-header h1 {{ margin:0; font-size:1.6rem; }}
    .badge {{ font-size:0.75rem; padding:0.25rem 0.6rem; border-radius:999px; }}
    .badge-active {{ background:rgba(34,197,94,0.15); color:#86efac; }}
    .badge-paused {{ background:rgba(251,191,36,0.15); color:#fde68a; }}
    .badge-completed {{ background:rgba(99,102,241,0.15); color:#c4b5fd; }}
    .badge-cancelled {{ background:rgba(239,68,68,0.15); color:#fca5a5; }}
    .stats {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(140px,1fr)); gap:1rem; margin-bottom:1.5rem; }}
    .stat-card {{ padding:1rem; border-radius:12px; background:var(--panel); border:1px solid var(--line); }}
    .stat-card .value {{ font-size:1.5rem; font-weight:900; }}
    .stat-card .label {{ font-size:0.78rem; color:var(--muted); }}
    .actions {{ display:flex; gap:0.5rem; flex-wrap:wrap; margin-bottom:1.5rem; }}
    .btn {{ display:inline-flex; align-items:center; padding:0.6rem 1rem; border-radius:5px; font-weight:700; cursor:pointer; border:0; font-size:0.88rem; }}
    .btn-success {{ background:rgba(34,197,94,0.2); color:#86efac; border:1px solid rgba(34,197,94,0.3); }}
    .btn-warning {{ background:rgba(251,191,36,0.2); color:#fde68a; border:1px solid rgba(251,191,36,0.3); }}
    .btn-danger {{ background:rgba(239,68,68,0.2); color:#fca5a5; border:1px solid rgba(239,68,68,0.3); }}
    table {{ width:100%; border-collapse:collapse; }}
    th, td {{ text-align:left; padding:0.7rem 0.5rem; border-bottom:1px solid var(--line); font-size:0.9rem; }}
    th {{ color:var(--muted); font-weight:600; }}
    tr:hover td {{ background:rgba(59,130,246,0.04); }}
    .service-tag {{ display:inline-block; font-size:0.78rem; padding:0.2rem 0.6rem; border-radius:999px; background:rgba(59,130,246,0.15); color:#93c5fd; margin-bottom:0.5rem; }}
    .detail-grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(200px,1fr)); gap:0.8rem; margin-bottom:1.5rem; padding:1rem; border-radius:12px; background:var(--panel); border:1px solid var(--line); }}
    .detail-item .dl {{ color:var(--muted); font-size:0.78rem; }}
    .detail-item .dd {{ font-weight:600; }}
</style>
</head>
<body>
<div class="shell">
    <div class="topbar">
        <a class="brand" href="/">VideoSync</a>
        <div class="nav-links">
            <a href="/campaigns">My Campaigns</a>
            <a href="/chat">Chat</a>
        </div>
    </div>

    <div class="campaign-header">
        <div>
            <div class="service-tag">{service_label}</div>
            <h1>{name}</h1>
        </div>
        <div>{status_badge}</div>
    </div>

    <div class="detail-grid">
        <div class="detail-item"><div class="dl">Brief</div><div class="dd">{brief}</div></div>
        <div class="detail-item"><div class="dl">Style</div><div class="dd">{style}</div></div>
        <div class="detail-item"><div class="dl">Duration</div><div class="dd">{duration}s</div></div>
        <div class="detail-item"><div class="dl">Posts/Day</div><div class="dd">{posts_per_day}</div></div>
        <div class="detail-item"><div class="dl">Start</div><div class="dd">{start_str}</div></div>
        <div class="detail-item"><div class="dl">End</div><div class="dd">{end_str}</div></div>
    </div>

    <div class="stats">
        <div class="stat-card"><div class="value">{total_published}</div><div class="label">Published</div></div>
        <div class="stat-card"><div class="value">{total_planned}</div><div class="label">Total Planned</div></div>
        <div class="stat-card"><div class="value">{posts_per_day}</div><div class="label">Posts / Day</div></div>
    </div>

    <div class="actions">
        {status_actions}
    </div>

    <h2>Post Calendar</h2>
    <table>
        <thead><tr><th>Day</th><th>Slot</th><th>Scheduled</th><th>Status</th><th>Media</th><th>Caption</th></tr></thead>
        <tbody>
            {posts_html}
        </tbody>
    </table>
</div>
<script>
async function updateStatus(newStatus) {{
    if (newStatus === 'cancelled' && !confirm('Cancel this campaign? This cannot be undone.')) return;
    const resp = await fetch('/api/campaigns/{id}/' + newStatus, {{ method: 'POST' }});
    const data = await resp.json();
    if (data.success) {{ location.reload(); }}
    else {{ alert('Failed: ' + (data.error || 'Unknown')); }}
}}
</script>
</body>
</html>"#
    ))
}

fn format_service_type(s: &str) -> &'static str {
    match s {
        "landing_page" => "SaaS Demo",
        "product_mockup" => "Product Mockup",
        "education" => "Education",
        "clipping" | "kick_auto_clipper" => "Clipping",
        "business_explainer" => "Business Explainer",
        "manim_explainer" => "Manim",
        "voice_audio" => "Voice & Audio",
        "full_stack" => "Full Stack",
        "whiteboard_animation" => "Whiteboard",
        "kinetic_typography" => "Kinetic Text",
        "animated_infographic" => "Infographic",
        "algorithm_viz" => "Algorithm Viz",
        "year_in_review" => "Year in Review",
        "isometric_explainer" => "Isometric",
        _ => s,
    }
}

pub async fn manim_explainer_page() -> Html<String> {
    Html(build_service_offer_page_html(
        "manim-explainer",
        "Manim Animated Explainer",
        "$75-$400+",
        "Send a topic, lesson, or script. Our AI agent generates a narrated Manim explainer with clean motion graphics, math/technical diagrams, and professional narration — in minutes, not hours.",
        "Built for educators, course creators, math/finance channels, SaaS teams, and anyone who needs clear animated explainers faster than traditional animation pipelines.",
        "The simple offer: send a topic, outline, lesson, script, or technical concept and get a narrated animated explainer video. The full pack includes the full explainer with motion graphics, diagrams, professional voiceover narration, captions, and a delivery page with downloads. Powered by Manim Community Edition — 5-20x faster render than Blender with CPU-friendly Cairo backend.",
         "/campaigns/new?service=manim_explainer",
         "Create campaign",
         "/chat",
         "Try one-off in chat",
        &[
            "Narrated animated explainer videos with clean motion graphics",
            "Math/technical diagrams, formulas, data visualizations, and process flows",
            "Professional voiceover narration via VibeVoice TTS (natural speech)",
            "Supports any duration: 30s shorts to 10+ minute lessons",
            "Captions and subtitles included",
            "Delivered as a review/download link you can share immediately",
            "5-20x faster than Blender pipeline — CPU-friendly Cairo backend",
        ],
        &[
            "Share the topic, lesson, script, or source material you want explained.",
            "Our AI agent plans the visual narrative and generates Manim Python code per scene.",
            "VideoSync renders each scene in parallel, stitches them together, adds narration.",
            "You receive a delivery page with preview and download links.",
        ],
        &[
            ("Educator or course creator", "Needs visual explainers for complex topics without spending days on animation."),
            ("Math/finance channel", "Needs clear animated diagrams and formula walkthroughs for technical content."),
            ("Product or SaaS team", "Needs a narrated explainer that makes a complex product or concept easy to understand."),
        ],
        r#"["manim_explainer","education","three_d_scene","voice_audio"]"#,
        r#"["manim_pack"]"#,
    ))
}

struct ServicePageTheme {
    accent: &'static str,
    secondary: &'static str,
    glow_a: &'static str,
    glow_b: &'static str,
    pattern: &'static str,
    eyebrow: &'static str,
    visual_title: &'static str,
    visual_points: &'static [&'static str],
    lab_class: &'static str,
}

fn service_page_theme(service_slug: &str) -> ServicePageTheme {
    match service_slug {
        "saas-launch-pack" => ServicePageTheme {
            accent: "#3b82f6",
            secondary: "#22c55e",
            glow_a: "rgba(59,130,246,0.24)",
            glow_b: "rgba(34,197,94,0.10)",
            pattern: "linear-gradient(135deg, rgba(59,130,246,0.26), transparent 38%), repeating-linear-gradient(90deg, rgba(147,197,253,0.10) 0 1px, transparent 1px 58px), repeating-linear-gradient(0deg, rgba(147,197,253,0.08) 0 1px, transparent 1px 42px)",
            eyebrow: "URL to demo",
            visual_title: "Website in. Buyer-ready product video out.",
            visual_points: &["Landing page scan", "Product story", "Mockups + motion", "Downloadable delivery"],
            lab_class: "service-lab-saas",
        },
        "thumbnail-hero-pack" | "clipper-enhancement-pack" => ServicePageTheme {
            accent: "#f97316",
            secondary: "#facc15",
            glow_a: "rgba(249,115,22,0.22)",
            glow_b: "rgba(250,204,21,0.12)",
            pattern: "radial-gradient(circle at 25% 30%, rgba(250,204,21,0.28), transparent 0 18%), radial-gradient(circle at 75% 24%, rgba(249,115,22,0.22), transparent 0 22%), linear-gradient(135deg, rgba(15,23,42,0.48), rgba(2,6,23,0.74))",
            eyebrow: "Click package",
            visual_title: "First frame, thumbnail, hook, and polish.",
            visual_points: &["Hero frame", "CTR thumbnail", "Caption/title card", "Reusable campaign visual"],
            lab_class: "service-lab-visual",
        },
        "product-mockup-pack" => ServicePageTheme {
            accent: "#14b8a6",
            secondary: "#38bdf8",
            glow_a: "rgba(20,184,166,0.20)",
            glow_b: "rgba(56,189,248,0.12)",
            pattern: "linear-gradient(135deg, rgba(20,184,166,0.24), transparent 42%), radial-gradient(circle at 70% 20%, rgba(56,189,248,0.18), transparent 0 24%), repeating-linear-gradient(135deg, rgba(226,232,240,0.07) 0 1px, transparent 1px 22px)",
            eyebrow: "Mockup system",
            visual_title: "Screenshots become animated product scenes.",
            visual_points: &["UI flow", "Device/browser scenes", "Callouts", "Ad-ready export"],
            lab_class: "service-lab-saas",
        },
        "education-explainer-pack" => ServicePageTheme {
            accent: "#22c55e",
            secondary: "#38bdf8",
            glow_a: "rgba(34,197,94,0.20)",
            glow_b: "rgba(56,189,248,0.10)",
            pattern: "linear-gradient(135deg, rgba(34,197,94,0.18), transparent 42%), repeating-linear-gradient(0deg, rgba(226,232,240,0.08) 0 1px, transparent 1px 34px), repeating-linear-gradient(90deg, rgba(226,232,240,0.06) 0 1px, transparent 1px 34px)",
            eyebrow: "Explain visually",
            visual_title: "Concept → storyboard → animated scenes → lesson.",
            visual_points: &["f(x) = clarity", "Animated diagrams", "Narrated lesson", "Long-form assembly"],
            lab_class: "service-lab-education",
        },
        "blender-scene-pack" => ServicePageTheme {
            accent: "#a855f7",
            secondary: "#fb7185",
            glow_a: "rgba(168,85,247,0.22)",
            glow_b: "rgba(251,113,133,0.12)",
            pattern: "radial-gradient(circle at 50% 28%, rgba(168,85,247,0.28), transparent 0 24%), conic-gradient(from 180deg at 50% 50%, rgba(251,113,133,0.12), rgba(59,130,246,0.18), rgba(168,85,247,0.12))",
            eyebrow: "Scene engine",
            visual_title: "3D scenes and cinematic support visuals.",
            visual_points: &["Scene brief", "Model/lighting", "Camera motion", "Rendered assets"],
            lab_class: "service-lab-visual",
        },
        "voice-audio-pack" => ServicePageTheme {
            accent: "#ec4899",
            secondary: "#8b5cf6",
            glow_a: "rgba(236,72,153,0.20)",
            glow_b: "rgba(139,92,246,0.14)",
            pattern: "repeating-linear-gradient(90deg, rgba(236,72,153,0.22) 0 3px, transparent 3px 18px), linear-gradient(135deg, rgba(15,23,42,0.62), rgba(88,28,135,0.36))",
            eyebrow: "Audio layer",
            visual_title: "Scripts, voiceovers, summaries, and narrated assets.",
            visual_points: &["Script polish", "Voiceover", "Audio-backed video", "Downloadable files"],
            lab_class: "service-lab-audio",
        },
        "kick-com-clipping" => ServicePageTheme {
            accent: "#00e701",
            secondary: "#8b5cf6",
            glow_a: "rgba(0,231,1,0.22)",
            glow_b: "rgba(139,92,246,0.14)",
            pattern: "linear-gradient(135deg, rgba(0,231,1,0.28), transparent 42%), radial-gradient(circle at 70% 80%, rgba(139,92,246,0.20), transparent 0 24%), repeating-linear-gradient(135deg, rgba(226,232,240,0.07) 0 1px, transparent 1px 22px)",
            eyebrow: "From Kick to clip",
            visual_title: "Kick.com VODs become ready-to-post highlights.",
            visual_points: &["VOD link", "Download + clip", "Captions + hook", "Delivery page"],
            lab_class: "service-lab-visual",
        },
        "mixed-agency-bundle" | "creator-manager-fulfillment" => ServicePageTheme {
            accent: "#0ea5e9",
            secondary: "#f59e0b",
            glow_a: "rgba(14,165,233,0.22)",
            glow_b: "rgba(245,158,11,0.12)",
            pattern: "linear-gradient(135deg, rgba(14,165,233,0.22), transparent 42%), radial-gradient(circle at 80% 15%, rgba(245,158,11,0.18), transparent 0 22%), repeating-linear-gradient(90deg, rgba(226,232,240,0.07) 0 1px, transparent 1px 46px)",
            eyebrow: "Agency system",
            visual_title: "Multiple client deliverables, one production backend.",
            visual_points: &["3 client URLs", "3 videos", "Delivery pages", "Monthly upsell"],
            lab_class: "service-lab-saas",
        },
        _ => ServicePageTheme {
            accent: "#3b82f6",
            secondary: "#22c55e",
            glow_a: "rgba(59,130,246,0.18)",
            glow_b: "rgba(34,197,94,0.10)",
            pattern: "linear-gradient(135deg, rgba(59,130,246,0.20), rgba(15,23,42,0.44))",
            eyebrow: "Production path",
            visual_title: "Brief in. Buyer-facing media out.",
            visual_points: &["Brief", "Workflow", "Review", "Delivery"],
            lab_class: "service-lab-saas",
        },
    }
}

fn build_services_overview_page_html() -> String {
    let launch_cards = [
        (
            "SaaS Demo Video",
            "/services/saas-launch-pack",
            "$399-$1,200+",
            "For SaaS founders and product teams who need their OWN product video — from a URL, screenshots, or brief. One polished demo for your launch.",
            "Website/app URL, screenshots, loom, or short brief",
            "Polished demo, promo, or walkthrough video autonomously. 30-120s is standard; longer available.",
        ),
        (
            "Agency Client Pack (3 videos)",
            "/services/mixed-agency-bundle",
            "$1,500 for 3 videos",
            "For agencies who want to RESELL video deliverables to existing clients. Send 3 client URLs and get 3 white-label videos back.",
            "3 client websites, offers, or landing pages",
            "3 client-ready demo/promo videos with delivery pages and download links.",
        ),
        (
            "Product Mockup Video",
            "/services/product-mockup-pack",
            "$299-$900+",
            "For founders and teams that need UI mockups, app-flow visuals, or product scenes before they have polished footage.",
            "Screenshots, product URL, Figma references, or app flow",
            "Animated UI/product mockups for ads, landing pages, demos, or sales decks.",
        ),
    ];
    let content_cards = [
        (
            "Education Explainer",
            "/services/education-explainer-pack",
            "$300-$1,500+",
            "For educators, technical creators, coaches, and course sellers who need clearer visual explanations.",
            "Topic, outline, lesson, script, or source material",
            "Animated diagrams, narrated explainers, visual lessons, or long-form educational videos.",
        ),
        (
            "Thumbnail & Hero Visual",
            "/services/thumbnail-hero-pack",
            "$75-$300+",
            "For creators, SaaS launches, ads, and landing pages that need a stronger first impression.",
            "Topic, product, face/photo, brand colors, or campaign goal",
            "Thumbnails, hero images, ad visuals, and campaign graphics ready to publish.",
        ),
        (
            "Clip Enhancement",
            "/services/clipper-enhancement-pack",
            "$250-$1,200+",
            "For creators and brands that already have clips but need them packaged like professional social content.",
            "Raw clips, highlights, timestamps, or exported videos",
            "Captions, title cards, lower thirds, thumbnails, motion graphics, and export-ready variants.",
        ),
        (
            "Kick.com Clipping",
            "/services/kick-com-clipping",
            "$75-$300+",
            "For Kick streamers, editors, and clip channels who need professional highlight clips from Kick VODs.",
            "Kick.com VOD or channel URL",
            "Extracted highlight clips with captions, thumbnails, hook cards, and delivery page.",
        ),
        (
            "Manim Animated Explainer",
            "/services/manim-explainer",
            "$75-$400+",
            "For educators, course creators, and technical channels who need narrated animated explainers with math diagrams and motion graphics.",
            "Topic, lesson, script, or technical concept",
            "Manim-powered animated explainers with voiceover narration, diagrams, and captions — 5-20x faster than Blender.",
        ),
        (
            "Social Publishing",
            "/admin/zernio",
            "$15/mo DIY · $147-$899 DFY",
            "For creators, clippers, and agencies who need automated cross-platform clip publishing via Zernio.",
            "Connected social accounts + content",
            "Schedule and auto-publish clips to YouTube, TikTok, Instagram, and X from one dashboard with Campaign Engine.",
        ),
    ];
    let production_cards = [
        (
            "3D/2D Animation Scene",
            "/services/blender-scene-pack",
            "$500-$2,500+",
            "For teams that need cinematic product visuals, 3D explainers, animated models, or support scenes.",
            "Idea, product, object, style reference, or scene description",
            "3D/2D animated visuals, product scenes, motion graphics, or explainer assets.",
        ),
        (
            "Voice & Audio Production",
            "/services/voice-audio-pack",
            "$99-$750+",
            "For videos, explainers, summaries, podcasts, and sales assets that need clean narration or audio.",
            "Script, topic, article, video, or rough notes",
            "Narration, voiceovers, podcast-style audio, summaries, or audio-backed video packages.",
        ),
    ];
    let agency_cards = [
        (
            "Agency Production Backend",
            "/services/creator-manager-fulfillment",
            "$1,500-$5,000+/mo",
            "For agencies and operators selling monthly video deliverables who need a reliable production layer.",
            "Recurring client tasks, briefs, brand notes, and fulfillment requirements",
            "Ongoing video, thumbnail, mockup, voice, and delivery-page work behind the scenes.",
        ),
        (
            "Programmable Payments & Asset API",
            "/services/x402-asset-api",
            "Custom / pay per call",
            "For technical teams that want wallet-paid delivery unlocks, paid previews, or media-generation API flows.",
            "Access pattern, asset type, and buyer flow",
            "Paid delivery pages, x402-style unlocks, and API-backed media access.",
        ),
    ];

    fn render_group(cards: &[(&str, &str, &str, &str, &str, &str)], group_eyebrow: &str) -> String {
        let items: String = cards.iter().map(|(title, href, price, audience, input, output)| {
            format!(
                r#"<article class="offer-card">
                  <div class="card-top">
                    <div class="eyebrow">{group_eyebrow}</div>
                  </div>
                  <h2>{title}</h2>
                  <div class="price">{price}</div>
                  <p class="audience">{audience}</p>
                  <div class="mini-list">
                    <div><strong>You send:</strong> {input}</div>
                    <div><strong>We deliver:</strong> {output}</div>
                  </div>
                  <a class="cta" href="{href}">See details</a>
                </article>"#
            )
        }).collect();
        format!(
            r#"<div class="group-section">
              <div class="group-heading">
                <div class="group-eyebrow">{group_eyebrow}</div>
              </div>
              <div class="grid">{items}</div>
            </div>"#
        )
    }

    let launch_html = render_group(&launch_cards, "Launch Assets");
    let content_html = render_group(&content_cards, "Content & Social");
    let production_html = render_group(&production_cards, "Production");
    let agency_html = render_group(&agency_cards, "Agency");

    let all_cards = format!("{launch_html}{content_html}{production_html}{agency_html}");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Done-For-You Video Production | VideoSync</title>
  <style>
    :root {{
      --bg:#07111d;
      --panel:rgba(9,18,31,0.86);
      --line:rgba(148,163,184,0.16);
      --line-strong:rgba(96,165,250,0.28);
      --text:#e5eefb;
      --muted:#a8b8d3;
      --blue:#3b82f6;
      --green:#22c55e;
      --amber:#fbbf24;
    }}
    * {{ box-sizing:border-box; }}
    body {{ margin:0; font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; background:
      radial-gradient(circle at top left, rgba(59,130,246,0.18), transparent 28%),
      radial-gradient(circle at bottom right, rgba(34,197,94,0.10), transparent 30%),
      #07111d; color:var(--text); }}
    .shell {{ max-width:1180px; margin:0 auto; padding:32px 20px 72px; }}
    .topbar {{ display:flex; justify-content:space-between; align-items:center; gap:1rem; flex-wrap:wrap; margin-bottom:28px; }}
    .brand {{ color:#fff; text-decoration:none; font-weight:800; font-size:1.3rem; }}
    .toplinks {{ display:flex; gap:0.8rem; flex-wrap:wrap; }}
    .toplinks a {{ color:#c7d8f6; text-decoration:none; padding:0.65rem 1rem; border:1px solid rgba(148,163,184,0.2); border-radius:999px; background:rgba(8,15,28,0.75); }}
    .hero {{ padding:28px; border-radius:28px; background:linear-gradient(135deg, rgba(59,130,246,0.2), rgba(8,15,28,0.94)); border:1px solid rgba(96,165,250,0.22); box-shadow:0 24px 70px rgba(2,6,23,0.45); }}
    .hero h1 {{ margin:0; font-size:3.15rem; line-height:1.03; max-width:940px; }}
    .hero p {{ margin:1rem 0 0; max-width:860px; color:#b9c8df; font-size:1.08rem; }}
    .hero-actions {{ display:flex; flex-wrap:wrap; gap:0.8rem; margin-top:1.35rem; }}
    .hero-action {{ display:inline-flex; align-items:center; text-decoration:none; padding:0.8rem 1.05rem; border-radius:999px; border:1px solid var(--line-strong); background:rgba(15,23,42,0.72); color:#dbeafe; font-weight:800; }}
    .hero-action.primary {{ background:linear-gradient(135deg,#3b82f6,#2563eb); color:white; border-color:transparent; }}
    .speed-badge {{ display:inline-flex; align-items:center; gap:0.45rem; background:rgba(34,197,94,0.15); border:1px solid rgba(34,197,94,0.25); border-radius:999px; padding:0.4rem 0.75rem; color:#86efac; font-weight:700; font-size:0.82rem; }}
    .compare-row {{ display:flex; flex-wrap:wrap; gap:1.2rem; margin-top:1.15rem; padding:1rem; border-radius:16px; background:rgba(15,23,42,0.7); border:1px solid rgba(148,163,184,0.12); }}
    .compare-item {{ flex:1; min-width:130px; }}
    .compare-item .label {{ color:var(--muted); font-size:0.78rem; text-transform:uppercase; letter-spacing:0.06em; }}
    .compare-item .value {{ color:#fff; font-size:1.35rem; font-weight:900; }}
    .compare-item .note {{ color:#86efac; font-size:0.75rem; font-weight:700; }}
    .chooser {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(210px,1fr)); gap:0.85rem; margin-top:1.15rem; }}
    .chooser-card {{ border:1px solid rgba(96,165,250,0.18); background:rgba(15,23,42,0.58); border-radius:18px; padding:1rem; color:#dbeafe; }}
    .chooser-card strong {{ display:block; color:#fff; margin-bottom:0.3rem; }}
    .chooser-card span {{ color:#a8b8d3; font-size:0.92rem; line-height:1.4; }}
    .group-section {{ margin-top:2rem; }}
    .group-heading {{ margin-bottom:0.6rem; }}
    .group-eyebrow {{ color:#93c5fd; font-size:0.85rem; letter-spacing:0.08em; text-transform:uppercase; font-weight:800; }}
    .grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(280px,1fr)); gap:1.2rem; }}
    .offer-card {{ display:flex; flex-direction:column; min-height:360px; padding:1.45rem; border-radius:24px; background:var(--panel); border:1px solid var(--line); box-shadow:0 18px 45px rgba(2,6,23,0.35); }}
    .card-top {{ display:flex; justify-content:space-between; align-items:center; gap:0.8rem; }}
    .offer-card h2 {{ margin:0.55rem 0 0.35rem; font-size:1.5rem; line-height:1.22; }}
    .eyebrow {{ color:#93c5fd; font-size:0.78rem; text-transform:uppercase; letter-spacing:0.08em; font-weight:800; }}
    .price {{ font-size:1.85rem; font-weight:900; margin:0.55rem 0 0.7rem; color:#fff; }}
    .audience {{ color:#b9c8df; min-height:76px; line-height:1.5; font-size:0.94rem; }}
    .mini-list {{ display:grid; gap:0.75rem; margin-top:0.3rem; color:#a8b8d3; font-size:0.94rem; line-height:1.45; }}
    .mini-list strong {{ color:#e5eefb; }}
    .cta {{ display:inline-flex; align-self:flex-start; margin-top:auto; color:#fff; text-decoration:none; background:#2563eb; border-radius:5px; padding:0.82rem 1.2rem; font-weight:800; transition:transform 0.18s ease, box-shadow 0.18s ease, background 0.18s ease; }}
    .cta:hover {{ transform:translateY(-2px); box-shadow:0 14px 34px rgba(37,99,235,0.26); background:#1d4ed8; }}
    .how-section {{ margin-top:2rem; padding:1.5rem; border-radius:24px; background:var(--panel); border:1px solid var(--line); }}
    .how-section h2 {{ margin:0 0 1rem; font-size:1.6rem; }}
    .how-grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(140px,1fr)); gap:0.8rem; }}
    .how-step {{ text-align:center; padding:0.8rem; border-radius:18px; background:rgba(15,23,42,0.6); border:1px solid rgba(148,163,184,0.1); }}
    .how-step .step-num {{ display:inline-flex; align-items:center; justify-content:center; width:36px; height:36px; border-radius:50%; background:linear-gradient(135deg,#3b82f6,#2563eb); color:#fff; font-weight:900; font-size:1rem; margin-bottom:0.4rem; }}
    .how-step .step-label {{ color:#dbeafe; font-weight:700; }}
    .how-step .step-desc {{ color:var(--muted); font-size:0.82rem; }}
    .lead-cta {{ margin-top:2rem; padding:1.8rem; border-radius:24px; background:linear-gradient(135deg, rgba(59,130,246,0.2), rgba(8,15,28,0.94)); border:1px solid rgba(96,165,250,0.3); text-align:center; }}
    .lead-cta h2 {{ margin:0 0 0.5rem; font-size:1.8rem; }}
    .lead-cta p {{ color:var(--muted); max-width:600px; margin:0 auto 1rem; }}
    .lead-form {{ display:flex; flex-wrap:wrap; gap:0.6rem; justify-content:center; }}
    .lead-form input {{ flex:1; min-width:200px; max-width:320px; padding:0.75rem 1rem; border-radius:999px; border:1px solid var(--line-strong); background:rgba(15,23,42,0.8); color:#fff; outline:none; }}
    .lead-form button {{ padding:0.75rem 1.5rem; border-radius:999px; border:none; background:linear-gradient(135deg,#3b82f6,#2563eb); color:#fff; font-weight:800; cursor:pointer; }}
    @media (max-width: 760px) {{
      .hero h1 {{ font-size:2.25rem; }}
      .offer-card {{ min-height:auto; }}
    }}
  </style>
</head>
<body>
  <div class="shell">
    <div class="topbar">
      <a class="brand" href="/">VideoSync</a>
      <div class="toplinks" id="homepageAuthButtons">
        <a href="/dashboard">Dashboard</a>
        <a href="/chat">Chat</a>
        <a href="/subscribe">Subscribe</a>
        <a href="/api-access">API Access</a>
      </div>
    </div>
    <section class="hero">
      <div class="eyebrow">Done-For-You Services</div>
      <h1>AI-Generated Video in Hours — at a Fraction of Agency Cost</h1>
      <p>No human editor. No back-and-forth. No waiting weeks. Our AI agent autonomously produces your deliverable from a URL, screenshot, brief, or raw clips — and you get a polished delivery page in hours, not weeks.</p>
      <div style="display:flex;flex-wrap:wrap;gap:0.6rem;margin-top:0.8rem">
        <span class="speed-badge">⚡ 24-48 hour delivery</span>
        <span class="speed-badge">🤖 No human in the loop</span>
        <span class="speed-badge">📄 Delivery page included</span>
      </div>
      <div class="compare-row">
        <div class="compare-item">
          <div class="label">Agency Cost</div>
          <div class="value" style="color:#f87171">$1,000–$5,000+</div>
          <div class="note">Human editor, 2-4 week turnaround</div>
        </div>
        <div class="compare-item">
          <div class="label">VideoSync</div>
          <div class="value" style="color:#86efac">$75–$2,500</div>
          <div class="note">AI agent, 24-48 hour delivery</div>
        </div>
        <div class="compare-item">
          <div class="label">You Save</div>
          <div class="value" style="color:#fbbf24">50–90%</div>
          <div class="note">No project manager, no editor markup</div>
        </div>
      </div>
      <div class="hero-actions">
        <a class="hero-action primary" href="/services/saas-launch-pack">Start with a SaaS demo</a>
        <a class="hero-action" href="/services/mixed-agency-bundle">See agency pack</a>
        <a class="hero-action" href="/chat?prompt=I%20want%20a%20done-for-you%20video%20service.%20Help%20me%20choose%20the%20right%20pack.&autosend=1">Help me choose</a>
      </div>
      <div class="chooser">
        <div class="chooser-card"><strong>Have a URL?</strong><span>SaaS demo, website walkthrough, or product mockup.</span></div>
        <div class="chooser-card"><strong>Have raw clips?</strong><span>Clip enhancement, thumbnails, or social cutdowns.</span></div>
        <div class="chooser-card"><strong>Have a lesson?</strong><span>Education explainer with animated visuals, diagrams, and narration.</span></div>
        <div class="chooser-card"><strong>Need something custom?</strong><span>Animated scene, voice/audio, or agency backend.</span></div>
      </div>
    </section>
    <section class="how-section">
      <h2>How It Works</h2>
      <div class="how-grid">
        <div class="how-step"><div class="step-num">1</div><div class="step-label">Send Your Brief</div><div class="step-desc">URL, screenshot, clips, or a short description</div></div>
        <div class="how-step"><div class="step-num">2</div><div class="step-label">AI Plans</div><div class="step-desc">Agent selects tools, sources assets, builds storyboard</div></div>
        <div class="how-step"><div class="step-num">3</div><div class="step-label">AI Produces</div><div class="step-desc">Animated scenes, voiceovers, edits, effects — fully automated</div></div>
        <div class="how-step"><div class="step-num">4</div><div class="step-label">QA Review</div><div class="step-desc">Multimodal AI checks quality against your brief</div></div>
        <div class="how-step"><div class="step-num">5</div><div class="step-label">Delivery Page</div><div class="step-desc">Polished preview with download + share links</div></div>
      </div>
    </section>
    <section class="group-section">
      <div class="group-heading">
        <div class="group-eyebrow">Pick the outcome — our AI agent handles the rest</div>
      </div>
      {all_cards}
    </section>
    <section class="lead-cta">
      <h2>Not Sure Which Pack Fits Your Project?</h2>
      <p>Describe what you need and we'll recommend the right service — or build a custom quote.</p>
      <form class="lead-form" action="/api/prospect/register" method="POST">
        <input type="text" name="description" placeholder="e.g. 90-second SaaS demo video with voiceover" required>
        <input type="email" name="email" placeholder="your@email.com" required>
        <button type="submit">Get a Recommendation</button>
      </form>
    </section>
  </div>
<script>
class DynamicBackgroundManager {{
    constructor() {{
        this.lastUpdate = Date.now();
        this.interval = 5 * 60 * 1000;
        this.init();
    }}
    async init() {{
        await this.updateBg();
        setInterval(() => this.updateBg(), this.interval);
    }}
    async updateBg() {{
        try {{
            const r = await fetch('/api/background/image');
            if (!r.ok) return;
            const ct = r.headers.get('content-type') || '';
            if (ct.includes('application/json')) {{
                const d = await r.json();
                if (d.fallback && d.gradient) document.body.style.background = d.gradient;
                return;
            }}
            const blob = await r.blob();
            const url = URL.createObjectURL(blob);
            const o = document.createElement('div');
            o.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;background-image:url('+url+');background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none';
            document.body.appendChild(o);
            setTimeout(() => o.style.opacity = '0.3', 100);
            setTimeout(() => {{
                const old = document.querySelectorAll('div[style*="background-image"]');
                old.forEach((e,i) => {{ if (i < old.length - 1) e.remove(); }});
            }}, 1100);
        }} catch(e) {{ console.error(e); }}
    }}
}}
new DynamicBackgroundManager();
</script>
<script>
(async function(){{
  const t=localStorage.getItem('authToken')||localStorage.getItem('admin_token')||localStorage.getItem('auth_token');
  const c=document.getElementById('homepageAuthButtons');
  if(t&&c)try{{
    const r=await fetch('/api/auth/verify',{{headers:{{'Authorization':'Bearer '+t}}}});
    if(r.ok){{
      const d=await r.json(),u=d.user||d;
      c.innerHTML='<span style="color:var(--muted);margin-right:8px">'+(u.email||u.username||'User')+'</span><a href="/dashboard" class="btn btn-secondary">Dashboard</a><a href='#' onclick="localStorage.clear();location.reload()" class="btn btn-secondary">Logout</a>';
    }}
  }}catch(e){{}}
}})();
</script>
</body>
</html>"#
    )
}

fn build_x402_docs_page_html() -> String {
    let endpoint_cards = [
        (
            "GET /api/subscribe/unlock-spec",
            "Returns the signed payment requirements needed to start the creator subscription flow.",
        ),
        (
            "POST /api/subscribe/unlock",
            "Accepts the signed payment authorization and activates the paid subscription.",
        ),
        (
            "GET /api/api-access/unlock-spec",
            "Returns the payment requirements for API access tiers and usage plans.",
        ),
        (
            "POST /api/api-access/unlock",
            "Settles the API tier payment and unlocks the selected access tier.",
        ),
        (
            "GET /delivery/:id/unlock-spec",
            "Returns the HD delivery unlock price and payment requirements for a preview page.",
        ),
        (
            "POST /delivery/:id/unlock",
            "Settles the delivery unlock payment and returns the HD access metadata.",
        ),
    ]
    .into_iter()
    .map(|(route, copy)| {
        format!(
            r#"<article class="endpoint-card"><div class="route">{route}</div><p>{copy}</p></article>"#
        )
    })
    .collect::<Vec<_>>()
    .join("");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Programmable Payments | VideoSync</title>
  <style>
    :root {{
      --bg:#07111d;
      --panel:rgba(9,18,31,0.84);
      --line:rgba(148,163,184,0.16);
      --line-strong:rgba(96,165,250,0.28);
      --text:#e5eefb;
      --muted:#a8b8d3;
      --blue:#3b82f6;
      --green:#22c55e;
      --shadow:0 24px 70px rgba(2,6,23,0.45);
    }}
    * {{ box-sizing:border-box; }}
    body {{ margin:0; font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; color:var(--text); background:
      radial-gradient(circle at top left, rgba(59,130,246,0.18), transparent 28%),
      linear-gradient(135deg, #0a1322 0%, #0d1728 55%, #07111d 100%); }}
    .shell {{ max-width:1180px; margin:0 auto; padding:28px 20px 72px; }}
    .topbar {{ display:flex; justify-content:space-between; align-items:center; gap:1rem; flex-wrap:wrap; margin-bottom:24px; }}
    .brand {{ color:#fff; text-decoration:none; font-size:1.35rem; font-weight:800; }}
    .toplinks {{ display:flex; gap:0.8rem; flex-wrap:wrap; }}
    .toplinks a {{ text-decoration:none; color:#dbeafe; padding:0.65rem 1rem; border-radius:999px; border:1px solid var(--line); background:rgba(8,15,28,0.76); }}
    .hero, .panel {{ border-radius:24px; border:1px solid var(--line); background:var(--panel); box-shadow:var(--shadow); backdrop-filter: blur(16px); }}
    .hero {{ padding:1.8rem; }}
    .eyebrow {{ color:#93c5fd; font-size:0.8rem; letter-spacing:0.08em; text-transform:uppercase; font-weight:800; }}
    h1 {{ margin:0.6rem 0 0; font-size:3rem; line-height:1.05; }}
    p {{ color:var(--muted); }}
    .cta-row {{ display:flex; gap:0.8rem; flex-wrap:wrap; margin-top:1.4rem; }}
    .btn {{ display:inline-flex; align-items:center; justify-content:center; padding:0.8rem 1.2rem; border-radius:999px; text-decoration:none; font-weight:700; }}
    .btn-primary {{ background:linear-gradient(135deg,#3b82f6,#2563eb); color:#fff; }}
    .btn-secondary {{ background:rgba(15,23,42,0.7); border:1px solid var(--line-strong); color:#dbeafe; }}
    .panel {{ padding:1.5rem; margin-top:1.2rem; }}
    .grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(260px,1fr)); gap:1rem; }}
    .endpoint-card, .code-card {{ padding:1.15rem; border-radius:18px; background:rgba(15,23,42,0.72); border:1px solid rgba(148,163,184,0.14); }}
    .route {{ font-family:ui-monospace,SFMono-Regular,Menlo,monospace; color:#bfdbfe; font-weight:700; margin-bottom:0.55rem; }}
    pre {{ margin:0; white-space:pre-wrap; font-family:ui-monospace,SFMono-Regular,Menlo,monospace; font-size:13px; color:#dbeafe; }}
    ul {{ margin:0; padding-left:1.1rem; color:#d7e3f5; }}
    li {{ margin:0.55rem 0; }}
    .spec-grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(300px,1fr)); gap:1rem; }}
    @media (max-width: 880px) {{
      h1 {{ font-size:2.3rem; }}
    }}
  </style>
</head>
<body>
  <div class="shell">
    <div class="topbar">
      <a class="brand" href="/">VideoSync</a>
      <div class="toplinks" id="homepageAuthButtons">
        <a href="/services">All Services</a>
        <a href="/api-access">API Access</a>
        <a href="/subscribe">Subscribe</a>
        <a href="/chat">Chat</a>
      </div>
    </div>

    <section class="hero">
      <div class="eyebrow">Programmable Payments</div>
      <h1>Wallet-paid delivery and API access for technical buyers</h1>
      <p>Use VideoSync when you need paid media unlocks or API access that can be purchased directly inside a product flow. The payment layer is designed for teams selling assets, previews, generation endpoints, or delivery access without forcing every buyer through a traditional checkout funnel.</p>
      <div class="cta-row">
        <a class="btn btn-primary" href="/api-access">Open API Access Page</a>
        <a class="btn btn-secondary" href="/chat?prompt=I%20want%20to%20integrate%20VideoSync%20through%20x402.%20Show%20me%20the%20best%20live%20payment%20flow%20for%20my%20use%20case.&autosend=1">Request an integration walkthrough</a>
      </div>
    </section>

    <section class="panel">
      <div class="eyebrow">Available now</div>
      <h2>Endpoints you can integrate today</h2>
      <div class="grid">
        {endpoint_cards}
      </div>
    </section>

    <section class="panel">
      <div class="eyebrow">Implementation patterns</div>
      <h2>What teams sell with this setup</h2>
      <ul>
        <li>Sell paid delivery unlocks for HD videos, previews, downloadable assets, and client handoff pages.</li>
        <li>Turn generation endpoints into paid developer products without adding a separate billing layer first.</li>
        <li>Support partner and automation flows where payment, access, and delivery need to happen in one request path.</li>
      </ul>
    </section>

    <section class="panel">
      <div class="eyebrow">Payment flow</div>
      <h2>How the unlock flow works</h2>
      <ul>
        <li>The client requests an `unlock-spec` endpoint first.</li>
        <li>VideoSync returns the payment requirements for a Base USDC transfer.</li>
        <li>The wallet signs the payment authorization.</li>
        <li>The signed payload is sent back in `X-Payment` to the matching `unlock` endpoint.</li>
        <li>VideoSync settles the payment and returns the unlocked resource or access metadata.</li>
      </ul>
    </section>

    <section class="panel">
      <div class="eyebrow">Integration basics</div>
      <h2>Headers and request shape</h2>
      <div class="spec-grid">
        <article class="code-card">
          <div class="route">Request headers</div>
<pre>Content-Type: application/json
X-Payment: &lt;signed x402 payload&gt;</pre>
        </article>
        <article class="code-card">
          <div class="route">Example resource pattern</div>
<pre>GET  /delivery/:id/unlock-spec
POST /delivery/:id/unlock

GET  /api/subscribe/unlock-spec
POST /api/subscribe/unlock</pre>
        </article>
      </div>
    </section>

    <section class="panel">
      <div class="eyebrow">Custom integration patterns</div>
      <h2>Revenue flows available through implementation work</h2>
      <ul>
        <li>`POST /api/x402/generate-thumbnail-pack`</li>
        <li>`POST /api/x402/generate-product-mockup`</li>
        <li>`POST /api/x402/generate-landing-hero-video`</li>
        <li>`POST /api/x402/generate-narrated-explainer`</li>
        <li>`POST /api/x402/enrich-creator-lead`</li>
      </ul>
      <p>If you need one of these paid flows immediately, we can wire it through the current payment and delivery infrastructure as a custom integration while the dedicated endpoint is finalized.</p>
    </section>

    <section class="panel">
      <div class="eyebrow">Why teams choose it</div>
      <h2>Why this works as a monetization layer</h2>
      <ul>
        <li>It gives technical teams a direct path to sell access, unlocks, and paid delivery without adding a heavy checkout layer to every workflow.</li>
        <li>It fits one-off asset delivery, API access, and paid generation endpoints where speed and automation matter.</li>
        <li>It supports partner integrations and developer-led buying flows while the main service pages stay focused on buyer outcomes instead of payment mechanics.</li>
      </ul>
    </section>
  </div>
<script>
(async function(){{
  const t=localStorage.getItem('authToken')||localStorage.getItem('admin_token')||localStorage.getItem('auth_token');
  const c=document.getElementById('homepageAuthButtons');
  if(t&&c)try{{
    const r=await fetch('/api/auth/verify',{{headers:{{'Authorization':'Bearer '+t}}}});
    if(r.ok){{
      const d=await r.json(),u=d.user||d;
      c.innerHTML='<span style="color:var(--muted);margin-right:8px">'+(u.email||u.username||'User')+'</span><a href="/dashboard" class="btn btn-secondary">Dashboard</a><a href='#' onclick="localStorage.clear();location.reload()" class="btn btn-secondary">Logout</a>';
    }}
  }}catch(e){{}}
}})();
</script>
</body>
</html>"#
    )
}

fn build_service_offer_page_html(
    service_slug: &str,
    title: &str,
    price: &str,
    tagline: &str,
    audience: &str,
    summary: &str,
    primary_href: &str,
    primary_label: &str,
    secondary_href: &str,
    secondary_label: &str,
    includes: &[&str],
    workflow: &[&str],
    lead_samples: &[(&str, &str)],
    sample_filters_json: &str,
    paypal_offers: &str,
) -> String {
    let hero_highlights_html = includes
        .iter()
        .take(3)
        .map(|item| format!(r#"<div class="hero-highlight">{item}</div>"#))
        .collect::<Vec<_>>()
        .join("");
    let includes_html = includes
        .iter()
        .map(|item| format!(r#"<li>{item}</li>"#))
        .collect::<Vec<_>>()
        .join("");
    let workflow_html = workflow
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            format!(
                r#"<div class="step"><div class="step-no">{}</div><div>{}</div></div>"#,
                idx + 1,
                item
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let lead_html = lead_samples
        .iter()
        .map(|(persona, copy)| {
            format!(
                r#"<article class="lead-card"><div class="lead-title">{persona}</div><p>{copy}</p></article>"#
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let nav_html = service_page_nav(title);
    let sample_filters_json = sample_filters_json.to_string();
    let sample_ui = service_sample_ui_config(service_slug).to_string();
    let theme = service_page_theme(service_slug);
    let theme_accent = theme.accent;
    let theme_secondary = theme.secondary;
    let theme_glow_a = theme.glow_a;
    let theme_glow_b = theme.glow_b;
    let theme_pattern = theme.pattern;
    let theme_eyebrow = theme.eyebrow;
    let theme_visual_title = theme.visual_title;
    let theme_lab_class = theme.lab_class;
    let visual_points = theme
        .visual_points
        .iter()
        .map(|point| format!(r#"<span>{point}</span>"#))
        .collect::<Vec<_>>()
        .join("");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{title} | VideoSync</title>
  <style>
    :root {{
      --bg:#07111d;
      --panel:rgba(9,18,31,0.84);
      --line:rgba(148,163,184,0.16);
      --line-strong:rgba(96,165,250,0.28);
      --text:#e5eefb;
      --muted:#a8b8d3;
      --blue:{theme_accent};
      --green:{theme_secondary};
      --shadow:0 24px 70px rgba(2,6,23,0.45);
    }}
    * {{ box-sizing:border-box; }}
    body {{ margin:0; font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; color:var(--text); background:
      radial-gradient(circle at top left, {theme_glow_a}, transparent 28%),
      radial-gradient(circle at bottom right, {theme_glow_b}, transparent 30%),
      linear-gradient(135deg, #0a1322 0%, #0d1728 55%, #07111d 100%); position:relative; overflow-x:hidden; }}
    a {{ color:inherit; }}
    body::before {{ content:""; position:fixed; inset:0; background:
      radial-gradient(circle at 15% 10%, rgba(56,189,248,0.08), transparent 0 28%),
      radial-gradient(circle at 85% 12%, rgba(34,197,94,0.08), transparent 0 22%),
      radial-gradient(circle at 50% 85%, rgba(99,102,241,0.10), transparent 0 24%);
      pointer-events:none; z-index:-2; }}
    .shell {{ max-width:1180px; margin:0 auto; padding:28px 20px 72px; }}
    .topbar {{ display:flex; justify-content:space-between; align-items:center; gap:1rem; flex-wrap:wrap; margin-bottom:24px; }}
    .brand {{ color:#fff; text-decoration:none; font-size:1.35rem; font-weight:800; }}
    .toplinks {{ display:flex; gap:0.8rem; flex-wrap:wrap; }}
    .toplinks a {{ text-decoration:none; color:#dbeafe; padding:0.65rem 1rem; border-radius:999px; border:1px solid var(--line); background:rgba(8,15,28,0.76); }}
    .page-content {{ position:relative; z-index:1; }}
    .hero {{ display:grid; grid-template-columns:minmax(0,1.1fr) minmax(300px,0.9fr); gap:1.2rem; animation:riseIn 0.65s ease both; }}
    .hero-panel, .panel {{ border-radius:24px; border:1px solid var(--line); background:var(--panel); box-shadow:var(--shadow); backdrop-filter: blur(16px); }}
    .hero-panel {{ padding:1.8rem; }}
    .eyebrow {{ color:#93c5fd; font-size:0.8rem; letter-spacing:0.08em; text-transform:uppercase; font-weight:800; }}
    h1 {{ margin:0.6rem 0 0; font-size:3rem; line-height:1.05; }}
    .tagline {{ margin-top:0.9rem; color:#dbeafe; font-size:1.08rem; }}
    .summary {{ margin-top:1rem; color:var(--muted); max-width:760px; }}
    .hero-highlights {{ display:flex; flex-wrap:wrap; gap:0.75rem; margin-top:1.15rem; }}
    .hero-highlight {{ padding:0.7rem 0.95rem; border-radius:8px; background:rgba(15,23,42,0.68); border:1px solid rgba(96,165,250,0.18); color:#dbeafe; font-size:0.92rem; line-height:1.35; }}
    .cta-row {{ display:flex; gap:0.8rem; flex-wrap:wrap; margin-top:1.4rem; }}
    .btn {{ display:inline-flex; align-items:center; justify-content:center; padding:0.8rem 1.2rem; border-radius:5px; text-decoration:none; font-weight:700; transition:transform 0.18s ease, box-shadow 0.18s ease, border-color 0.18s ease; cursor:pointer; }}
    .btn:hover {{ transform:translateY(-2px); box-shadow:0 14px 34px rgba(2,6,23,0.28); }}
    .btn-primary {{ background:linear-gradient(135deg,var(--blue),#2563eb); color:#fff; border:0; }}
    .btn-secondary {{ background:rgba(15,23,42,0.7); border:1px solid var(--line-strong); color:#dbeafe; }}
    .mini-metrics {{ display:grid; gap:0.9rem; padding:1.5rem; }}
    .metric {{ padding:1rem; border-radius:18px; background:rgba(15,23,42,0.72); border:1px solid rgba(148,163,184,0.14); }}
    .metric strong {{ display:block; font-size:1.9rem; }}
    .metric span {{ color:var(--muted); font-size:0.92rem; }}
    .service-nav {{ display:flex; flex-wrap:wrap; gap:0.7rem; margin:24px 0; }}
    .service-nav a {{ text-decoration:none; padding:0.7rem 1rem; border-radius:5px; border:1px solid var(--line); background:rgba(8,15,28,0.76); color:#dbeafe; transition:transform 0.18s ease, border-color 0.18s ease; }}
    .service-nav a:hover {{ transform:translateY(-2px); border-color:var(--line-strong); }}
    .service-nav a.active {{ border-color:var(--line-strong); background:rgba(59,130,246,0.16); }}
    .grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(280px,1fr)); gap:1.2rem; }}
    .panel {{ padding:1.5rem; animation:softReveal 0.7s ease both; }}
    .panel h2 {{ margin:0.5rem 0 0.9rem; font-size:1.45rem; }}
    .panel p {{ color:var(--muted); }}
    .checklist, .sample-list {{ list-style:none; padding:0; margin:0; }}
    .checklist li {{ position:relative; padding:0.45rem 0 0.45rem 1rem; color:#d7e3f5; }}
    .checklist li::before {{ content:""; position:absolute; left:0; top:0.95rem; width:6px; height:6px; border-radius:999px; background:var(--green); }}
    .step {{ display:flex; gap:0.85rem; align-items:flex-start; margin:0.85rem 0; }}
    .step-no {{ width:32px; height:32px; border-radius:999px; background:rgba(59,130,246,0.18); border:1px solid var(--line-strong); display:flex; align-items:center; justify-content:center; font-weight:800; flex-shrink:0; }}
    .lead-grid, .portfolio-grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(220px,1fr)); gap:1rem; }}
    .lead-card, .sample-card {{ padding:1.1rem; border-radius:18px; background:rgba(15,23,42,0.72); border:1px solid rgba(148,163,184,0.14); }}
    .lead-title, .sample-title {{ font-weight:800; color:#fff; margin-bottom:0.45rem; }}
    .sample-video {{ width:100%; aspect-ratio:16/9; object-fit:cover; border-radius:14px; background:#020617; margin-bottom:0.8rem; }}
    .sample-meta {{ color:var(--muted); font-size:0.92rem; }}
    .sample-actions {{ display:flex; gap:0.7rem; flex-wrap:wrap; margin-top:0.9rem; }}
    .sample-actions a {{ color:#93c5fd; text-decoration:none; font-weight:700; }}
    .sample-lab {{ display:grid; grid-template-columns:minmax(0,1fr) minmax(280px,0.8fr); gap:1rem; }}
    .visual-stage {{ min-height:100%; position:relative; overflow:hidden; }}
    .visual-stage::before {{ content:""; position:absolute; inset:-20%; background:{theme_pattern}; opacity:0.72; filter:blur(0.2px); animation:slowDrift 14s ease-in-out infinite alternate; }}
    .visual-inner {{ position:relative; z-index:1; min-height:320px; display:flex; flex-direction:column; justify-content:space-between; }}
    .visual-title {{ font-size:1.7rem; font-weight:900; letter-spacing:-0.03em; max-width:320px; }}
    .visual-points {{ display:grid; gap:0.65rem; }}
    .visual-points span {{ display:block; border:1px solid rgba(226,232,240,0.13); background:rgba(2,6,23,0.42); border-radius:10px; padding:0.75rem 0.85rem; color:#dbeafe; }}
    .service-saas-launch-pack .visual-inner {{ background:linear-gradient(180deg, rgba(15,23,42,0.25), rgba(15,23,42,0.68)); border-radius:18px; padding:1rem; }}
    .service-thumbnail-hero-pack .visual-points {{ grid-template-columns:repeat(2,minmax(0,1fr)); }}
    .service-thumbnail-hero-pack .visual-points span:first-child {{ grid-column:span 2; min-height:88px; font-size:1.15rem; display:flex; align-items:center; }}
    .service-education-explainer-pack .visual-stage {{ background:rgba(2,6,23,0.42); }}
    .service-education-explainer-pack .visual-points span {{ font-family:ui-monospace,SFMono-Regular,Menlo,monospace; }}
    .service-blender-scene-pack .visual-title {{ text-transform:uppercase; letter-spacing:0.08em; font-size:1.25rem; }}
    .service-voice-audio-pack .visual-points span {{ border-left:4px solid var(--green); }}
    .sample-lab-card.service-lab-saas {{ border-color:rgba(96,165,250,0.24); }}
    .sample-lab-card.service-lab-visual {{ background:linear-gradient(145deg, rgba(15,23,42,0.86), rgba(30,41,59,0.68)); }}
    .sample-lab-card.service-lab-education {{ background:linear-gradient(145deg, rgba(12,20,33,0.92), rgba(20,45,48,0.58)); }}
    .sample-lab-card.service-lab-audio {{ background:linear-gradient(145deg, rgba(15,23,42,0.88), rgba(68,27,99,0.42)); }}
    .sample-lab-card {{ padding:1.25rem; border-radius:18px; background:rgba(15,23,42,0.72); border:1px solid rgba(148,163,184,0.14); }}
    .sample-form {{ display:grid; gap:0.85rem; margin-top:0.9rem; }}
    .sample-form label {{ font-size:0.88rem; font-weight:700; color:#dbeafe; }}
    .sample-form input, .sample-form textarea {{ width:100%; border-radius:6px; border:1px solid rgba(96,165,250,0.22); background:rgba(2,6,23,0.56); color:#e5eefb; padding:0.9rem 1rem; font:inherit; transition:border-color 0.18s ease, box-shadow 0.18s ease; }}
    .sample-form input:focus, .sample-form textarea:focus {{ outline:none; border-color:var(--blue); box-shadow:0 0 0 3px rgba(59,130,246,0.16); }}
    .sample-form textarea {{ min-height:120px; resize:vertical; }}
    .sample-form .row {{ display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:0.8rem; }}
    .sample-note {{ color:var(--muted); font-size:0.92rem; line-height:1.6; }}
    .sample-status {{ margin-top:0.75rem; color:#cbd5e1; font-size:0.92rem; }}
    .admin-only-shell {{ display:none; }}
    .admin-only-shell.visible {{ display:block; }}
    .admin-pill {{ display:inline-flex; align-items:center; gap:0.45rem; padding:0.45rem 0.8rem; border-radius:999px; border:1px solid rgba(34,197,94,0.28); background:rgba(34,197,94,0.10); color:#bbf7d0; font-size:0.85rem; font-weight:700; }}
    .locked-pill {{ display:inline-flex; align-items:center; gap:0.45rem; padding:0.45rem 0.8rem; border-radius:999px; border:1px solid rgba(148,163,184,0.18); background:rgba(15,23,42,0.56); color:#cbd5e1; font-size:0.85rem; font-weight:700; }}
    .empty-note {{ color:var(--muted); }}
    @keyframes riseIn {{ from {{ opacity:0; transform:translateY(18px); }} to {{ opacity:1; transform:translateY(0); }} }}
    @keyframes softReveal {{ from {{ opacity:0; transform:translateY(12px); }} to {{ opacity:1; transform:translateY(0); }} }}
    @keyframes slowDrift {{ from {{ transform:translate3d(-1%, -1%, 0) scale(1.02); }} to {{ transform:translate3d(1.5%, 1%, 0) scale(1.06); }} }}
    @keyframes fadeScale {{ from {{ opacity:0; transform:scale(0.96); }} to {{ opacity:1; transform:scale(1); }} }}
    @keyframes slideUp {{ from {{ opacity:0; transform:translateY(24px); }} to {{ opacity:1; transform:translateY(0); }} }}
    @keyframes pulseGlow {{ 0%,100% {{ box-shadow:0 0 8px rgba(96,165,250,0.08); }} 50% {{ box-shadow:0 0 20px rgba(96,165,250,0.22); }} }}
    .panel {{ animation-delay:calc(var(--idx,0) * 0.08s); }}
    .checklist li {{ animation:slideUp 0.5s ease both; animation-delay:calc(var(--i,0) * 0.06s); }}
    .lead-card {{ animation:fadeScale 0.55s ease both; animation-delay:calc(var(--ci,0) * 0.07s); transition:transform 0.25s ease, border-color 0.25s ease, box-shadow 0.25s ease; }}
    .lead-card:hover {{ transform:translateY(-4px); border-color:rgba(96,165,250,0.32); box-shadow:0 12px 40px rgba(2,6,23,0.3); }}
    .step {{ animation:slideUp 0.5s ease both; animation-delay:calc(var(--si,0) * 0.07s); }}
    .hero-highlight {{ transition:transform 0.25s ease, border-color 0.25s ease, box-shadow 0.25s ease; }}
    .hero-highlight:hover {{ transform:translateY(-2px); border-color:rgba(96,165,250,0.35); box-shadow:0 8px 24px rgba(2,6,23,0.24); }}
    .metric {{ transition:transform 0.25s ease, border-color 0.25s ease; }}
    .metric:hover {{ transform:translateY(-3px); border-color:rgba(96,165,250,0.25); }}
    @media (max-width: 880px) {{
      .hero {{ grid-template-columns:1fr; }}
      h1 {{ font-size:2.4rem; }}
      .sample-lab {{ grid-template-columns:1fr; }}
      .sample-form .row {{ grid-template-columns:1fr; }}
    }}
  </style>
</head>
<body>
  <div class="shell page-content service-{service_slug}">
    <div class="topbar">
      <a class="brand" href="/">VideoSync</a>
      <div class="toplinks" id="homepageAuthButtons">
        <a href="/services">All Services</a>
        <a href="/dashboard">Dashboard</a>
        <a href="/chat">Chat</a>
        <a href="/subscribe">Subscribe</a>
        <a href="/api-access">API Access</a>
      </div>
    </div>

    <section class="hero">
      <div class="hero-panel">
        <div class="eyebrow">Service</div>
        <h1>{title}</h1>
        <div class="tagline">{tagline}</div>
        <p class="summary">{summary}</p>
        <div class="hero-highlights">{hero_highlights_html}</div>
        <div class="cta-row">
          <a class="btn btn-primary" href="{primary_href}">{primary_label}</a>
          <a class="btn btn-secondary" href="{secondary_href}">{secondary_label}</a>
        </div>
        <div id="paypal-section" style="margin-top:1.2rem;border-top:1px solid var(--line);padding-top:1rem;">
          <div style="font-size:0.85rem;color:var(--muted);margin-bottom:0.6rem;">Buy this service — Pay with PayPal or Credit Card:</div>
          <details style="margin-bottom:0.8rem;">
            <summary style="cursor:pointer;font-size:0.8rem;color:var(--muted);padding:0.3rem 0;">Billing address (optional)</summary>
            <div style="display:grid;grid-template-columns:1fr 1fr;gap:0.5rem;margin-top:0.5rem;">
              <input type="text" id="billing-name" placeholder="Full name" style="grid-column:1/-1;background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:0.5rem 0.7rem;color:var(--text);font-family:inherit;font-size:0.85rem;">
              <input type="text" id="billing-address1" placeholder="Address line 1" style="grid-column:1/-1;background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:0.5rem 0.7rem;color:var(--text);font-family:inherit;font-size:0.85rem;">
              <input type="text" id="billing-address2" placeholder="Address line 2 (optional)" style="grid-column:1/-1;background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:0.5rem 0.7rem;color:var(--text);font-family:inherit;font-size:0.85rem;">
              <input type="text" id="billing-city" placeholder="City" style="background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:0.5rem 0.7rem;color:var(--text);font-family:inherit;font-size:0.85rem;">
              <input type="text" id="billing-state" placeholder="State / Province" style="background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:0.5rem 0.7rem;color:var(--text);font-family:inherit;font-size:0.85rem;">
              <input type="text" id="billing-zip" placeholder="ZIP / Postal code" style="background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:0.5rem 0.7rem;color:var(--text);font-family:inherit;font-size:0.85rem;">
              <select id="billing-country" style="grid-column:1/-1;background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:0.5rem 0.7rem;color:var(--text);font-family:inherit;font-size:0.85rem;">
                <option value="US">United States</option>
                <option value="KE">Kenya</option>
                <option value="GB">United Kingdom</option>
                <option value="CA">Canada</option>
                <option value="AU">Australia</option>
                <option value="DE">Germany</option>
                <option value="FR">France</option>
                <option value="IN">India</option>
                <option value="NG">Nigeria</option>
                <option value="ZA">South Africa</option>
                <option value="OTHER">Other</option>
              </select>
            </div>
          </details>
          <div id="paypal-buttons-container"></div>
        </div>
      </div>
      <aside class="hero-panel visual-stage">
        <div class="visual-inner">
          <div>
            <div class="eyebrow">{theme_eyebrow}</div>
            <div class="visual-title">{theme_visual_title}</div>
          </div>
          <div class="visual-points">{visual_points}</div>
          <div class="metric"><strong>{price}</strong><span>Typical project range</span></div>
        </div>
      </aside>
    </section>

    {nav_html}

    <section class="grid">
      <article class="panel">
        <div class="eyebrow">What you get</div>
        <h2>Included in this service</h2>
        <ul class="checklist">{includes_html}</ul>
      </article>
      <article class="panel">
        <div class="eyebrow">Process</div>
        <h2>How the project runs</h2>
        {workflow_html}
      </article>
    </section>

    <section class="panel" style="margin-top:1.2rem;">
      <div class="eyebrow">Best fit</div>
      <h2>Who this is for</h2>
      <div class="lead-grid">{lead_html}</div>
    </section>

    <section class="panel" style="margin-top:1.2rem;">
      <div class="eyebrow">Request output</div>
      <h2 id="sampleSectionTitle">Request a custom output</h2>
      <p id="sampleSectionCopy">Describe the exact output you want and VideoSync will hand the brief to the agent. Your included outputs stay available until the upgrade CTA takes over.</p>
      <div class="sample-lab" style="margin-top:1rem;">
        <div class="sample-lab-card {theme_lab_class}">
          <div class="locked-pill" id="sampleGateBadge">5 outputs available</div>
          <form id="sampleRequestForm" class="sample-form">
            <div class="row">
              <div>
                <label for="sampleUrl" id="sampleUrlLabel">Reference URL or media link</label>
                <input id="sampleUrl" type="text" placeholder="https://example.com, YouTube/Twitch URL, or asset link">
              </div>
              <div>
                <label for="sampleContact" id="sampleContactLabel">Prospect or brand name</label>
                <input id="sampleContact" type="text" placeholder="Client, brand, or creator name">
              </div>
            </div>
            <div class="row">
              <div>
                <label for="sampleFormat" id="sampleFormatLabel">Requested format</label>
                <input id="sampleFormat" type="text" placeholder="Video type, length, or asset format">
              </div>
              <div>
                <label for="sampleOutcome" id="sampleOutcomeLabel">Goal</label>
                <input id="sampleOutcome" type="text" placeholder="What this sample should help you prove or sell">
              </div>
            </div>
            <div>
              <label for="sampleBrief" id="sampleBriefLabel">Describe the output you want the agent to create</label>
              <textarea id="sampleBrief" placeholder="Describe the output you want the agent to create."></textarea>
            </div>
            <div class="cta-row" style="margin-top:0;">
              <button type="submit" class="btn btn-primary" id="sampleLaunchBtn">Open project chat</button>
              <a class="btn btn-secondary" href="/subscribe" id="sampleUpgradeBtn" style="display:none;">Continue with paid output</a>
            </div>
          </form>
          <div class="sample-status" id="sampleStatus">This opens the agent chat with a structured brief so the requested output can be generated from scratch.</div>
        </div>
        <aside class="sample-lab-card {theme_lab_class}">
          <div class="eyebrow">Before you buy</div>
          <h2 style="margin-top:0.5rem;" id="sampleHelperTitle">How generation works</h2>
          <p class="sample-note" id="sampleHelperCopy">Use each included output to test the production direction before you pay for more.</p>
          <ul class="checklist" style="margin-top:0.8rem;">
            <li id="sampleHelperBullet1">Describe what you want clearly.</li>
            <li id="sampleHelperBullet2">The agent opens in chat with your structured brief.</li>
            <li id="sampleHelperBullet3">Your included outputs stay available before checkout appears.</li>
          </ul>
          <div class="sample-lab-card" style="margin-top:1rem; padding:1rem 1rem 0.95rem;">
            <div class="eyebrow" id="sampleExampleHeading">Example request</div>
            <p class="sample-note" id="sampleExampleCopy" style="margin-top:0.55rem;">Create a polished buyer-facing sample that shows the requested output clearly and includes a usable delivery or review link.</p>
          </div>
        </aside>
      </div>
    </section>

    <section class="panel admin-only-shell" id="adminReviewShell" style="margin-top:1.2rem;">
      <div class="eyebrow">Admin review only</div>
      <div style="display:flex;justify-content:space-between;align-items:center;gap:1rem;flex-wrap:wrap;">
        <div>
          <h2>Live delivery examples from the platform</h2>
          <p>These cards stay visible only for staff/superusers while the public-facing sample flow is being rebuilt.</p>
        </div>
        <div class="admin-pill">Admin visibility enabled</div>
      </div>
      <div id="portfolioGrid" class="portfolio-grid" style="margin-top:1rem;">
        <div class="empty-note">Loading internal review samples...</div>
      </div>
    </section>
  </div>

  <script>
    const sampleFilters = {sample_filters_json};
    const fallbackOrigin = window.location.origin;
    const serviceSlug = {service_slug:?};
    const sampleUi = {sample_ui};

    function applySampleUiConfig() {{
      document.getElementById('sampleSectionTitle').textContent = sampleUi.section_title;
      document.getElementById('sampleSectionCopy').textContent = sampleUi.section_copy;
      document.getElementById('sampleUrlLabel').textContent = sampleUi.source_label;
      document.getElementById('sampleUrl').placeholder = sampleUi.source_placeholder;
      document.getElementById('sampleContactLabel').textContent = sampleUi.contact_label;
      document.getElementById('sampleContact').placeholder = sampleUi.contact_placeholder;
      document.getElementById('sampleFormatLabel').textContent = sampleUi.format_label;
      document.getElementById('sampleFormat').placeholder = sampleUi.format_placeholder;
      document.getElementById('sampleOutcomeLabel').textContent = sampleUi.outcome_label;
      document.getElementById('sampleOutcome').placeholder = sampleUi.outcome_placeholder;
      document.getElementById('sampleBriefLabel').textContent = sampleUi.brief_label;
      document.getElementById('sampleBrief').placeholder = sampleUi.brief_placeholder;
      document.getElementById('sampleLaunchBtn').textContent = sampleUi.launch_label;
      document.getElementById('sampleUpgradeBtn').href = sampleUi.upgrade_href;
      document.getElementById('sampleUpgradeBtn').textContent = sampleUi.upgrade_label;
      document.getElementById('sampleStatus').textContent = sampleUi.status_idle;
      document.getElementById('sampleHelperTitle').textContent = sampleUi.helper_title;
      document.getElementById('sampleHelperCopy').textContent = sampleUi.helper_copy;
      document.getElementById('sampleHelperBullet1').textContent = sampleUi.helper_bullets[0] || '';
      document.getElementById('sampleHelperBullet2').textContent = sampleUi.helper_bullets[1] || '';
      document.getElementById('sampleHelperBullet3').textContent = sampleUi.helper_bullets[2] || '';
      document.getElementById('sampleExampleHeading').textContent = sampleUi.example_heading;
      document.getElementById('sampleExampleCopy').textContent = sampleUi.example_request;
    }}

    function absoluteUrl(value) {{
      if (!value) return '';
      if (/^https?:\/\//i.test(value)) return value;
      return `${{fallbackOrigin}}${{value.startsWith('/') ? value : `/${{value}}`}}`;
    }}

    function parseJwt(token) {{
      try {{
        const base64 = token.split('.')[1];
        if (!base64) return null;
        const normalized = base64.replace(/-/g, '+').replace(/_/g, '/');
        return JSON.parse(atob(normalized));
      }} catch (_) {{
        return null;
      }}
    }}

    function getAuthToken() {{
      return localStorage.getItem('authToken')
        || localStorage.getItem('admin_token')
        || localStorage.getItem('auth_token')
        || '';
    }}

    function isAdminUser() {{
      const token = getAuthToken();
      const claims = token ? parseJwt(token) : null;
      return !!(claims && (claims.is_staff || claims.is_superuser));
    }}

    function draftStorageKey() {{
      return `videosync:service-sample-draft:${{serviceSlug}}`;
    }}

    async function updateSampleGateUI() {{
      const launchBtn = document.getElementById('sampleLaunchBtn');
      const upgradeBtn = document.getElementById('sampleUpgradeBtn');
      const badge = document.getElementById('sampleGateBadge');
      const authToken = getAuthToken();
      if (!authToken) {{
        badge.textContent = sampleUi.anon_badge;
        launchBtn.style.display = 'inline-flex';
        launchBtn.disabled = false;
        upgradeBtn.style.display = 'none';
        return;
      }}

      try {{
        const response = await fetch(`/api/service-samples/quota?service=${{encodeURIComponent(serviceSlug)}}`, {{
          headers: {{
            Authorization: `Bearer ${{authToken}}`
          }}
        }});
        const payload = await response.json();
        const remaining = Number(payload.remaining || 0);
        if (payload.unlimited) {{
          badge.textContent = 'Admin or staff access: launch limit bypassed';
          launchBtn.style.display = 'inline-flex';
          launchBtn.disabled = false;
          upgradeBtn.style.display = 'none';
          return;
        }}

        if (remaining > 0) {{
          const unit = remaining === 1 ? sampleUi.included_unit_singular : sampleUi.included_unit_plural;
          badge.textContent = `${{remaining}} included ${{unit}} remaining before checkout`;
          launchBtn.style.display = 'inline-flex';
          launchBtn.disabled = false;
          upgradeBtn.style.display = 'none';
        }} else {{
          badge.textContent = sampleUi.limit_reached_badge;
          launchBtn.style.display = 'none';
          upgradeBtn.style.display = 'inline-flex';
        }}
      }} catch (_) {{
        badge.textContent = 'Sample quota could not be loaded right now.';
        launchBtn.style.display = 'none';
        upgradeBtn.style.display = 'inline-flex';
      }}
    }}

    function sampleMatches(sample) {{
      const bucket = [
        sample.gig_type,
        sample.portfolio_category,
        sample.title,
        sample.company,
        sample.sales_positioning,
      ]
        .filter(Boolean)
        .join(' ')
        .toLowerCase();
      return sampleFilters.some((filter) => bucket.includes(String(filter).toLowerCase()));
    }}

    function renderSampleCard(sample) {{
      const media = sample.output_r2_url || sample.preview_r2_url || '';
      const mediaHtml = media
        ? `<video class="sample-video" src="${{media}}" controls preload="metadata"></video>`
        : '';
      const authToken = getAuthToken();
      const rawDeliveryUrl = absoluteUrl(sample.public_delivery_url || sample.delivery_url || '');
      const deliveryUrl = rawDeliveryUrl && authToken
        ? `${{rawDeliveryUrl}}${{rawDeliveryUrl.includes('?') ? '&' : '?'}}token=${{encodeURIComponent(authToken)}}`
        : rawDeliveryUrl;
      const sourceUrl = sample.source_url ? absoluteUrl(sample.source_url) : '';
      const downloadUrl = absoluteUrl(sample.output_r2_url || sample.preview_r2_url || '');
      const downloadName = (sample.output_filename || sample.title || sample.company || 'videosync-sample')
        .toString()
        .replace(/[^a-z0-9._-]+/gi, '-')
        .replace(/^-+|-+$/g, '')
        || 'videosync-sample';
      return `
        <article class="sample-card">
          ${{mediaHtml}}
          <div class="sample-title">${{sample.company || sample.title || 'Portfolio sample'}}</div>
          <div class="sample-meta">${{sample.title || ''}}</div>
          <div class="sample-meta" style="margin-top:0.35rem;">Status: ${{sample.status || 'unknown'}}</div>
          <div class="sample-actions">
            ${{deliveryUrl ? `<a href="${{deliveryUrl}}" target="_blank" rel="noreferrer">Delivery</a>` : ''}}
            ${{downloadUrl ? `<a href="${{downloadUrl}}" download="${{downloadName}}" target="_blank" rel="noreferrer">Download</a>` : ''}}
            ${{sourceUrl ? `<a href="${{sourceUrl}}" target="_blank" rel="noreferrer">Source</a>` : ''}}
          </div>
        </article>
      `;
    }}

    function loadAdminPortfolioSamples() {{
      if (!isAdminUser()) return;
      const adminShell = document.getElementById('adminReviewShell');
      adminShell.classList.add('visible');
      const authToken = getAuthToken();
      fetch('/api/portfolio-samples', {{
        headers: authToken ? {{ Authorization: `Bearer ${{authToken}}` }} : {{}}
      }})
        .then((response) => response.json())
        .then((payload) => {{
          const root = document.getElementById('portfolioGrid');
          const rawSamples = Array.isArray(payload.samples) ? payload.samples : [];
          const completed = rawSamples.filter((sample) => sample.status === 'completed');
          const prioritized = completed.filter(sampleMatches);
          const selected = (prioritized.length ? prioritized : completed).slice(0, 4);
          if (!selected.length) {{
            root.innerHTML = '<div class="empty-note">No completed internal review samples are available yet. Use the chat or dashboard to generate a fresh delivery sample.</div>';
            return;
          }}
          root.innerHTML = selected.map(renderSampleCard).join('');
        }})
        .catch(() => {{
          document.getElementById('portfolioGrid').innerHTML =
            '<div class="empty-note">Admin review samples could not be loaded right now.</div>';
        }});
    }}

    function storePendingSampleDraft() {{
      const draft = {{
        reference_url: document.getElementById('sampleUrl').value.trim(),
        prospect_name: document.getElementById('sampleContact').value.trim(),
        format: document.getElementById('sampleFormat').value.trim(),
        outcome: document.getElementById('sampleOutcome').value.trim(),
        brief: document.getElementById('sampleBrief').value.trim(),
      }};
      sessionStorage.setItem(draftStorageKey(), JSON.stringify(draft));
    }}

    function loadPendingSampleDraft() {{
      try {{
        const raw = sessionStorage.getItem(draftStorageKey());
        return raw ? JSON.parse(raw) : null;
      }} catch (_) {{
        return null;
      }}
    }}

    function clearPendingSampleDraft() {{
      sessionStorage.removeItem(draftStorageKey());
    }}

    async function submitSampleRequest(event) {{
      event.preventDefault();

      const url = document.getElementById('sampleUrl').value.trim();
      const contact = document.getElementById('sampleContact').value.trim();
      const formatValue = document.getElementById('sampleFormat').value.trim();
      const outcomeValue = document.getElementById('sampleOutcome').value.trim();
      const brief = document.getElementById('sampleBrief').value.trim();
      const status = document.getElementById('sampleStatus');
      const authToken = getAuthToken();

      if (!brief) {{
        status.textContent = 'Add a short brief so the agent knows what sample to generate.';
        return;
      }}

      if (!authToken) {{
        storePendingSampleDraft();
        status.textContent = 'Sign in first so the agent can launch your tracked custom sample session.';
        const redirectTo = `${{window.location.pathname}}?launch_sample=1`;
        window.location.href = `/login?redirect_to=${{encodeURIComponent(redirectTo)}}`;
        return;
      }}

      status.textContent = 'Recording your sample request and opening the AI chat...';

      const structuredBriefParts = [
        brief,
        formatValue ? `${{sampleUi.format_label}}: ${{formatValue}}` : '',
        outcomeValue ? `${{sampleUi.outcome_label}}: ${{outcomeValue}}` : ''
      ].filter(Boolean);
      const structuredBrief = structuredBriefParts.join('\n');

      try {{
        const response = await fetch('/api/service-samples/request', {{
          method: 'POST',
          headers: {{
            'Content-Type': 'application/json',
            Authorization: `Bearer ${{authToken}}`
          }},
          body: JSON.stringify({{
            service_slug: serviceSlug,
            reference_url: url || null,
            prospect_name: contact || null,
            brief: structuredBrief,
            source: 'videosync_service'
          }})
        }});
        const payload = await response.json();
        if (!payload.success) {{
          if (payload.limit_reached) {{
            status.textContent = payload.message || 'Included launches used up.';
            await updateSampleGateUI();
            return;
          }}
          status.textContent = payload.message || 'Sample request failed.';
          return;
        }}

        clearPendingSampleDraft();
        await updateSampleGateUI();
        status.textContent = 'Opening the AI chat with your structured sample brief...';
        window.location.href = payload.chat_url;
      }} catch (_) {{
        status.textContent = 'Failed to launch the sample request right now.';
      }}
    }}

    async function resumePendingSampleIfRequested() {{
      const params = new URLSearchParams(window.location.search);
      if (params.get('launch_sample') !== '1') return;
      const authToken = getAuthToken();
      if (!authToken) return;

      const draft = loadPendingSampleDraft();
      if (!draft || !draft.brief) return;

      document.getElementById('sampleUrl').value = draft.reference_url || '';
      document.getElementById('sampleContact').value = draft.prospect_name || '';
      document.getElementById('sampleFormat').value = draft.format || '';
      document.getElementById('sampleOutcome').value = draft.outcome || '';
      document.getElementById('sampleBrief').value = draft.brief || '';

      const fakeEvent = {{ preventDefault() {{}} }};
      await submitSampleRequest(fakeEvent);
    }}

    class ServicePageDynamicBackgroundManager {{
      constructor() {{
        this.lastBackgroundUpdate = Date.now();
        this.updateInterval = 5 * 60 * 1000;
        this.retryDelay = 30 * 1000;
        this.isUpdating = false;
        this.init();
      }}

      async init() {{
        await this.updateBackground();
        setInterval(() => this.checkAndUpdateBackground(), 60 * 1000);
      }}

      async checkAndUpdateBackground() {{
        if (this.isUpdating) return;
        if (Date.now() - this.lastBackgroundUpdate >= this.updateInterval) {{
          await this.updateBackground();
        }}
      }}

      async updateBackground() {{
        if (this.isUpdating) return;
        this.isUpdating = true;
        try {{
          const response = await fetch('/api/background/image');
          if (!response.ok) return;
          const contentType = response.headers.get('content-type') || '';
          if (contentType.includes('application/json')) {{
            const data = await response.json();
            if (data.fallback && data.gradient) {{
              document.body.style.background = data.gradient;
            }}
            this.lastBackgroundUpdate = Date.now();
            return;
          }}
          const blob = await response.blob();
          const imageUrl = URL.createObjectURL(blob);
          let overlay = document.getElementById('serviceDynamicBg');
          if (!overlay) {{
            overlay = document.createElement('div');
            overlay.id = 'serviceDynamicBg';
          overlay.style.cssText = 'position:fixed;inset:0;background-size:cover;background-position:center;background-attachment:fixed;opacity:0;transition:opacity 0.9s ease;z-index:0;pointer-events:none;mix-blend-mode:screen;';
            document.body.appendChild(overlay);
          }}
          overlay.style.backgroundImage = 'url(' + imageUrl + ')';
          requestAnimationFrame(() => {{
            overlay.style.opacity = '0.16';
          }});
          this.lastBackgroundUpdate = Date.now();
        }} catch (_) {{
          setTimeout(() => {{
            this.lastBackgroundUpdate = Date.now() - this.updateInterval + this.retryDelay;
          }}, this.retryDelay);
        }} finally {{
          this.isUpdating = false;
        }}
          this.lastBackgroundUpdate = Date.now();
          return;
        }}
        const blob = await response.blob();
          const blob = await response.blob();
          const imageUrl = URL.createObjectURL(blob);
          let overlay = document.getElementById('serviceDynamicBg');
          if (!overlay) {{
            overlay = document.createElement('div');
            overlay.id = 'serviceDynamicBg';
          overlay.style.cssText = 'position:fixed;inset:0;background-size:cover;background-position:center;background-attachment:fixed;opacity:0;transition:opacity 0.9s ease;z-index:0;pointer-events:none;mix-blend-mode:screen;';
            document.body.appendChild(overlay);
          }}
          overlay.style.backgroundImage = 'url(' + imageUrl + ')';
          requestAnimationFrame(() => {{
            overlay.style.opacity = '0.16';
          }});
          this.lastBackgroundUpdate = Date.now();
        }} catch (_) {{
          setTimeout(() => {{
            this.lastBackgroundUpdate = Date.now() - this.updateInterval + this.retryDelay;
          }}, this.retryDelay);
        }} finally {{
          this.isAdvanced = false;
          this.isUpdating = false;
        }}
      }}
    }}

    document.getElementById('sampleRequestForm').addEventListener('submit', submitSampleRequest);
    applySampleUiConfig();
    updateSampleGateUI();
    loadAdminPortfolioSamples();
    resumePendingSampleIfRequested();
    try {{
      new ServicePageDynamicBackgroundManager();
    }} catch (_) {{}}

    // ── Payment buttons (PayPal + USDC) ────────────────────────────────────
    (function() {{
      const offers = {paypal_offers};
      if (!offers || offers.length === 0) return;

      var container = document.getElementById('paypal-buttons-container');

      function showSuccess(deliveryId) {{
        var url = deliveryId ? '/delivery/' + deliveryId : '/dashboard';
        container.innerHTML =
          '<div style="padding:1rem;border-radius:8px;background:rgba(34,197,94,0.12);border:1px solid rgba(34,197,94,0.28);color:#bbf7d0;text-align:center;">' +
          'Payment successful! Your delivery is being prepared. <a href="' + url + '" style="color:#93c5fd;font-weight:700;">View delivery</a>' +
          '</div>';
      }}

      function setStatus(msg, isError) {{
        var el = document.getElementById('crypto-status');
        if (el) {{
          el.style.color = isError ? '#f87171' : '#9999bb';
          el.textContent = msg;
        }}
      }}

      // ── Fetch offer prices from unlock-spec ─────────────────────────────────
      var offerPrices = {{}};
      Promise.all(offers.map(function(offerId) {{
        return fetch('/api/crypto/unlock-spec', {{
          method: 'POST',
          headers: {{ 'Content-Type': 'application/json' }},
          body: JSON.stringify({{ offer_id: offerId }})
        }}).then(function(r) {{ return r.json(); }})
          .then(function(d) {{
            if (d.success) offerPrices[offerId] = d.price_usd_cents;
          }}).catch(function() {{}});
      }})).then(function() {{
        offers.forEach(function(offerId) {{
          var cents = offerPrices[offerId] || 0;
          var dollars = (cents / 100).toFixed(2);

          // Card
          var card = document.createElement('div');
          card.style.cssText = 'margin-bottom:1rem;padding:0.75rem;border-radius:8px;border:1px solid var(--line);';
          var label = document.createElement('div');
          label.style.cssText = 'font-size:0.85rem;color:var(--muted);margin-bottom:0.5rem;font-weight:600;';
          label.textContent = '$' + dollars + ' — ' + offerId.replace(/-/g, ' ');
          card.appendChild(label);

          var row = document.createElement('div');
          row.style.cssText = 'display:flex;gap:0.5rem;flex-wrap:wrap;';

          // PayPal button container
          var ppEl = document.createElement('div');
          ppEl.id = 'paypal-button-' + offerId;
          ppEl.style.flex = '1';
          ppEl.style.minWidth = '200px';
          row.appendChild(ppEl);

          // USDC button
          var cryptoBtn = document.createElement('button');
          cryptoBtn.textContent = 'Buy with USDC $' + dollars;
          cryptoBtn.style.cssText = 'flex:1;min-width:200px;padding:0.6rem 1rem;border-radius:999px;border:1px solid rgba(148,163,184,0.3);background:rgba(15,23,42,0.6);color:#dbeafe;cursor:pointer;font-size:0.85rem;white-space:nowrap;';
          cryptoBtn.onmouseenter = function() {{ cryptoBtn.style.background = 'rgba(99,102,241,0.2)'; }};
          cryptoBtn.onmouseleave = function() {{ cryptoBtn.style.background = 'rgba(15,23,42,0.6)'; }};
          cryptoBtn.dataset.offerId = offerId;
          row.appendChild(cryptoBtn);

          var statusEl = document.createElement('div');
          statusEl.id = 'crypto-status-' + offerId;
          statusEl.style.cssText = 'font-size:0.8rem;margin-top:0.3rem;';
          statusEl.textContent = '';
          card.appendChild(row);
          card.appendChild(statusEl);
          container.appendChild(card);

          // ── USDC click handler ──────────────────────────────────────────
          cryptoBtn.addEventListener('click', async function() {{
            var btn = this;
            var sid = 'crypto-status-' + offerId;
            var st = document.getElementById(sid);
            function set(m, e) {{ st.style.color = e ? '#f87171' : '#9999bb'; st.textContent = m; }}

            btn.disabled = true;
            btn.textContent = 'Connecting wallet...';

            // Step 1 — fetch unlock spec
            var spec;
            try {{
              var r = await fetch('/api/crypto/unlock-spec', {{
                method: 'POST',
                headers: {{ 'Content-Type': 'application/json' }},
                body: JSON.stringify({{ offer_id: offerId }})
              }});
              if (!r.ok) throw new Error('unlock-spec returned ' + r.status);
              spec = await r.json();
            }} catch (e) {{
              set('Could not fetch payment spec: ' + e.message, true);
              btn.disabled = false; btn.textContent = 'Buy with USDC $' + dollars;
              return;
            }}

            var req = (spec.x402 && spec.x402.accepts) ? spec.x402.accepts[0] : null;
            if (!req) {{ set('Payment spec missing requirements.', true); btn.disabled = false; btn.textContent = 'Buy with USDC $' + dollars; return; }}

            // Step 2 — detect wallet
            var provider = (window.phantom && window.phantom.ethereum) || window.ethereum;
            if (!provider) {{
              set('No crypto wallet detected. Install Phantom, MetaMask, or Coinbase Wallet.', true);
              btn.disabled = false; btn.textContent = 'Buy with USDC $' + dollars;
              return;
            }}

            // Step 3 — connect accounts + switch to Base
            var accounts;
            try {{ accounts = await provider.request({{ method: 'eth_requestAccounts' }}); }} catch (e) {{
              set('Wallet connection rejected.', true);
              btn.disabled = false; btn.textContent = 'Buy with USDC $' + dollars;
              return;
            }}
            var from = accounts[0];

            try {{
              await provider.request({{ method: 'wallet_switchEthereumChain', params: [{{ chainId: '0x2105' }}] }});
            }} catch (sw) {{
              try {{
                await provider.request({{
                  method: 'wallet_addEthereumChain',
                  params: [{{
                    chainId: '0x2105', chainName: 'Base',
                    nativeCurrency: {{ name: 'Ether', symbol: 'ETH', decimals: 18 }},
                    rpcUrls: ['https://mainnet.base.org'],
                    blockExplorerUrls: ['https://basescan.org']
                  }}]
                }});
              }} catch {{
                set('Switch your wallet to Base network and try again.', true);
                btn.disabled = false; btn.textContent = 'Buy with USDC $' + dollars;
                return;
              }}
            }}

            set('Sign the USDC payment in your wallet to unlock...');
            btn.textContent = 'Awaiting signature...';

            // Step 4 — build EIP-3009 typed data
            var validAfter  = 0;
            var validBefore = Math.floor(Date.now() / 1000) + (req.maxTimeoutSeconds || 120);
            var nonce = '0x' + Array.from(crypto.getRandomValues(new Uint8Array(32)))
                                       .map(function(b) {{ return b.toString(16).padStart(2, '0'); }}).join('');
            var typedData = {{
              types: {{
                EIP712Domain: [
                  {{ name: 'name', type: 'string' }},
                  {{ name: 'version', type: 'string' }},
                  {{ name: 'chainId', type: 'uint256' }},
                  {{ name: 'verifyingContract', type: 'address' }},
                ],
                TransferWithAuthorization: [
                  {{ name: 'from', type: 'address' }},
                  {{ name: 'to', type: 'address' }},
                  {{ name: 'value', type: 'uint256' }},
                  {{ name: 'validAfter', type: 'uint256' }},
                  {{ name: 'validBefore', type: 'uint256' }},
                  {{ name: 'nonce', type: 'bytes32' }},
                ],
              }},
              primaryType: 'TransferWithAuthorization',
              domain: {{
                name: (req.extra && req.extra.name) || 'USD Coin',
                version: (req.extra && req.extra.version) || '2',
                chainId: 8453,
                verifyingContract: req.asset,
              }},
              message: {{
                from: from, to: req.payTo,
                value: req.maxAmountRequired,
                validAfter: validAfter, validBefore: validBefore, nonce: nonce,
              }},
            }};

            var signature;
            try {{
              signature = await provider.request({{
                method: 'eth_signTypedData_v4',
                params: [from, JSON.stringify(typedData)]
              }});
            }} catch (e) {{
              set('Signature rejected.', true);
              btn.disabled = false; btn.textContent = 'Buy with USDC $' + dollars;
              return;
            }}

            // Step 5 — submit
            set('Submitting payment to Base network...');
            btn.textContent = 'Settling on-chain...';

            var xPaymentBody = {{
              x402Version: 1,
              scheme: req.scheme,
              network: req.network,
              payload: {{
                signature: signature,
                authorization: {{
                  from: from, to: req.payTo,
                  value: req.maxAmountRequired,
                  validAfter: String(validAfter),
                  validBefore: String(validBefore),
                  nonce: nonce,
                }},
              }},
            }};
            var xPaymentB64 = btoa(JSON.stringify(xPaymentBody));

            try {{
              var r = await fetch('/api/crypto/unlock', {{
                method: 'POST',
                headers: {{ 'Content-Type': 'application/json', 'X-Payment': xPaymentB64 }},
                body: JSON.stringify({{ offer_id: offerId }})
              }});
              var data = await r.json();
              if (!r.ok || !data.success) throw new Error(data.error || ('HTTP ' + r.status));
              var deliveryId = data.delivery_id || null;
              set(''); btn.textContent = '✅ Paid!';
              setTimeout(function() {{ showSuccess(deliveryId); }}, 600);
            }} catch (e) {{
              set('Settlement failed: ' + e.message, true);
              btn.disabled = false; btn.textContent = 'Buy with USDC $' + dollars;
            }}
          }});
        }});
      }});

      // ── PayPal buttons ──────────────────────────────────────────────────
      fetch('/api/paypal/config')
        .then(function(r) {{ return r.json(); }})
        .then(function(config) {{
          if (!config.client_id) return;
          var script = document.createElement('script');
          script.src = 'https://www.paypal.com/sdk/js?client-id=' + encodeURIComponent(config.client_id) + '&currency=USD&enable-funding=card';
          document.body.appendChild(script);
          script.onload = function() {{
            offers.forEach(function(offerId) {{
              paypal.Buttons({{
                style: {{ layout: 'horizontal', label: 'paypal', tagline: false, height: 40, color: 'blue', shape: 'pill' }},
                createOrder: function(data, actions) {{
                  return fetch('/api/paypal/orders', {{
                    method: 'POST',
                    headers: {{ 'Content-Type': 'application/json' }},
                    body: JSON.stringify({{ offer_id: offerId }})
                  }}).then(function(r) {{ return r.json(); }}).then(function(d) {{
                    if (!d.order || !d.order.id) throw new Error('No order ID');
                    return d.order.id;
                  }});
                }},
                onApprove: function(data, actions) {{
                  return fetch('/api/paypal/orders/' + data.orderID + '/capture', {{
                    method: 'POST'
                  }}).then(function(r) {{ return r.json(); }}).then(function(d) {{
                    if (d.success) showSuccess(d.delivery_id);
                    else alert('Payment capture failed: ' + (d.message || 'unknown error'));
                  }});
                }},
                onError: function(err) {{
                  console.error('PayPal error:', err);
                }}
              }}).render('#paypal-button-' + offerId);
            }});
          }};
        }}).catch(function(e) {{ console.error('PayPal config fetch failed:', e); }});
    }})();
  </script>
<script>
(async function(){{
  const t=localStorage.getItem('authToken')||localStorage.getItem('admin_token')||localStorage.getItem('auth_token');
  const c=document.getElementById('homepageAuthButtons');
  if(t&&c)try{{
    const r=await fetch('/api/auth/verify',{{headers:{{'Authorization':'Bearer '+t}}}});
    if(r.ok){{
      const d=await r.json(),u=d.user||d;
      c.innerHTML='<span style="color:var(--muted);margin-right:8px">'+(u.email||u.username||'User')+'</span><a href="/dashboard" class="btn btn-secondary">Dashboard</a><a href='#' onclick="localStorage.clear();location.reload()" class="btn btn-secondary">Logout</a>';
    }}
  }}catch(e){{}}
}})();
</script>
</body>
</html>"#
    )
}

fn service_page_nav(active_title: &str) -> String {
    let items = [
        ("SaaS Launch Pack", "/services/saas-launch-pack"),
        (
            "Thumbnail & Motion Graphics Pack",
            "/services/clipper-enhancement-pack",
        ),
        (
            "Agency Production Backend",
            "/services/creator-manager-fulfillment",
        ),
        ("Programmable Payments", "/services/x402-asset-api"),
    ];

    let links = items
        .into_iter()
        .map(|(label, href)| {
            let active = if label == active_title { "active" } else { "" };
            format!(r#"<a class="{active}" href="{href}">{label}</a>"#)
        })
        .collect::<Vec<_>>()
        .join("");

    format!(r#"<nav class="service-nav">{links}</nav>"#)
}

#[allow(dead_code)]
fn build_landing_page_html() -> &'static str {
    r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>🎬 VideoSync - AI-Powered Video Editing</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        html {
            scroll-behavior: smooth;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            line-height: 1.6;
            color: #e8e8e8;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-size: cover;
            background-position: center;
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
        }

        .container {
            max-width: 1200px;
            margin: 0 auto;
            padding: 0 20px;
        }

        /* Header */
        .header {
            background: rgba(26, 26, 46, 0.9);
            backdrop-filter: blur(10px);
            border-bottom: 1px solid rgba(59, 130, 246, 0.3);
            padding: 1rem 0;
            position: fixed;
            width: 100%;
            top: 0;
            z-index: 1000;
        }

        .nav {
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .logo {
            font-size: 1.5rem;
            font-weight: bold;
            color: white;
            text-decoration: none;
        }

        .nav-links {
            display: flex;
            gap: 2rem;
        }

        .nav-links a {
            color: white;
            text-decoration: none;
            padding: 0.5rem 1rem;
            border-radius: 20px;
            transition: background-color 0.3s;
        }

        .nav-links a:hover {
            background-color: rgba(59, 130, 246, 0.3);
        }

        .auth-buttons {
            display: flex;
            gap: 1rem;
        }

        .btn {
            padding: 0.75rem 1.5rem;
            border: none;
            border-radius: 25px;
            font-weight: 600;
            text-decoration: none;
            display: inline-block;
            transition: all 0.3s;
            cursor: pointer;
        }

        .btn-primary {
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            border: 1px solid rgba(59, 130, 246, 0.3);
        }

        .btn-primary:hover {
            background: linear-gradient(135deg, #2563eb, #1e40af);
            transform: translateY(-2px);
            box-shadow: 0 4px 20px rgba(59, 130, 246, 0.4);
        }

        .btn-secondary {
            background: rgba(30, 30, 52, 0.8);
            color: #e8e8e8;
            border: 2px solid rgba(59, 130, 246, 0.3);
        }

        .btn-secondary:hover {
            background: rgba(59, 130, 246, 0.2);
            border-color: rgba(59, 130, 246, 0.6);
        }

        /* Hero Section */
        .hero {
            padding: 120px 0 80px;
            text-align: center;
            color: white;
        }

        .hero h1 {
            font-size: 3.5rem;
            margin-bottom: 1.5rem;
            font-weight: 700;
            opacity: 0;
            transform: translateY(30px);
            animation: fadeInUp 0.8s ease-out 0.2s forwards;
        }

        .hero p {
            font-size: 1.3rem;
            margin-bottom: 2.5rem;
            opacity: 0;
            max-width: 600px;
            margin-left: auto;
            margin-right: auto;
            transform: translateY(30px);
            animation: fadeInUp 0.8s ease-out 0.4s forwards;
        }

        .hero-buttons {
            display: flex;
            gap: 1.5rem;
            justify-content: center;
            flex-wrap: wrap;
            opacity: 0;
            transform: translateY(30px);
            animation: fadeInUp 0.8s ease-out 0.6s forwards;
        }

        .btn-large {
            padding: 1rem 2rem;
            font-size: 1.1rem;
        }

        /* Features Section */
        .features {
            padding: 80px 0;
            background: rgba(15, 20, 25, 0.95);
            backdrop-filter: blur(20px);
        }

        .features h2 {
            text-align: center;
            font-size: 2.5rem;
            margin-bottom: 3rem;
            color: #f8fafc;
        }

        .features-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 3rem;
            margin-top: 2rem;
        }

        .feature-card {
            text-align: center;
            padding: 2rem;
            border-radius: 15px;
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            backdrop-filter: blur(10px);
            box-shadow: 0 10px 30px rgba(0,0,0,0.3);
            transition: all 0.4s cubic-bezier(0.4, 0, 0.2, 1);
            transform: translateY(0);
            opacity: 0;
            animation: fadeInUp 0.6s ease-out forwards;
        }

        .feature-card:nth-child(1) { animation-delay: 0.1s; }
        .feature-card:nth-child(2) { animation-delay: 0.2s; }
        .feature-card:nth-child(3) { animation-delay: 0.3s; }
        .feature-card:nth-child(4) { animation-delay: 0.4s; }
        .feature-card:nth-child(5) { animation-delay: 0.5s; }
        .feature-card:nth-child(6) { animation-delay: 0.6s; }

        .feature-card:hover {
            transform: translateY(-8px) scale(1.02);
            box-shadow: 0 20px 40px rgba(59, 130, 246, 0.2);
            border-color: rgba(59, 130, 246, 0.4);
        }

        @keyframes fadeInUp {
            from {
                opacity: 0;
                transform: translateY(30px);
            }
            to {
                opacity: 1;
                transform: translateY(0);
            }
        }

        .feature-icon {
            font-size: 3rem;
            margin-bottom: 1rem;
            display: inline-block;
            transition: transform 0.3s ease;
        }

        .feature-card:hover .feature-icon {
            transform: scale(1.1) rotate(5deg);
        }

        .feature-card h3 {
            font-size: 1.5rem;
            margin-bottom: 1rem;
            color: #f8fafc;
        }

        .feature-card p {
            color: #cbd5e1;
            line-height: 1.6;
        }

        /* Tools Section */
        .tools {
            padding: 80px 0;
            background: rgba(26, 26, 46, 0.8);
            backdrop-filter: blur(20px);
        }

        .tools h2 {
            text-align: center;
            font-size: 2.5rem;
            margin-bottom: 3rem;
            color: #f8fafc;
        }

        .tools-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 2rem;
            margin-top: 2rem;
        }

        .tool-category {
            background: rgba(30, 30, 52, 0.7);
            border: 1px solid rgba(59, 130, 246, 0.2);
            backdrop-filter: blur(10px);
            padding: 2rem;
            border-radius: 10px;
            box-shadow: 0 5px 15px rgba(0,0,0,0.3);
            transition: all 0.3s ease;
            opacity: 0;
            transform: translateY(20px);
            animation: slideInUp 0.6s ease-out forwards;
        }

        .tool-category:nth-child(1) { animation-delay: 0.1s; }
        .tool-category:nth-child(2) { animation-delay: 0.2s; }
        .tool-category:nth-child(3) { animation-delay: 0.3s; }
        .tool-category:nth-child(4) { animation-delay: 0.4s; }
        .tool-category:nth-child(5) { animation-delay: 0.5s; }
        .tool-category:nth-child(6) { animation-delay: 0.6s; }

        .tool-category:hover {
            transform: translateY(-5px);
            box-shadow: 0 15px 30px rgba(59, 130, 246, 0.2);
            border-color: rgba(59, 130, 246, 0.4);
        }

        @keyframes slideInUp {
            from {
                opacity: 0;
                transform: translateY(30px);
            }
            to {
                opacity: 1;
                transform: translateY(0);
            }
        }

        .tool-category h3 {
            font-size: 1.3rem;
            margin-bottom: 1rem;
            color: #3b82f6;
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }

        .tool-list {
            list-style: none;
        }

        .tool-list li {
            padding: 0.3rem 0;
            color: #cbd5e1;
            position: relative;
            padding-left: 1rem;
        }

        .tool-list li::before {
            content: "✓";
            position: absolute;
            left: 0;
            color: #10b981;
            font-weight: bold;
        }

        /* About Section */
        .about {
            padding: 80px 0;
            background: rgba(15, 20, 25, 0.95);
            backdrop-filter: blur(20px);
        }

        .about h2 {
            text-align: center;
            font-size: 2.5rem;
            margin-bottom: 3rem;
            color: #f8fafc;
        }

        .about-content {
            display: grid;
            grid-template-columns: 2fr 1fr;
            gap: 4rem;
            align-items: start;
        }

        .about-text h3 {
            font-size: 1.5rem;
            margin-bottom: 1rem;
            color: #3b82f6;
        }

        .about-text p {
            margin-bottom: 2rem;
            line-height: 1.7;
            color: #cbd5e1;
        }

        .about-stats {
            display: flex;
            flex-direction: column;
            gap: 2rem;
        }

        .stat-item {
            text-align: center;
            padding: 1.5rem;
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            backdrop-filter: blur(10px);
            border-radius: 12px;
            transition: all 0.3s ease;
        }

        .stat-item:hover {
            transform: translateY(-3px);
            box-shadow: 0 8px 25px rgba(59, 130, 246, 0.2);
            border-color: rgba(59, 130, 246, 0.4);
        }

        .stat-number {
            font-size: 2.5rem;
            font-weight: bold;
            color: #3b82f6;
            margin-bottom: 0.5rem;
        }

        .stat-label {
            color: #cbd5e1;
            font-weight: 500;
        }

        /* CTA Section */
        .cta {
            padding: 80px 0;
            background: linear-gradient(135deg, #1a1a2e 0%, #3b82f6 50%, #1e40af 100%);
            text-align: center;
            color: white;
        }

        .cta h2 {
            font-size: 2.5rem;
            margin-bottom: 1rem;
        }

        .cta p {
            font-size: 1.2rem;
            margin-bottom: 2rem;
            opacity: 0.9;
        }

        /* Privacy Section */
        .privacy-section {
            padding: 80px 0;
            background: rgba(26, 26, 46, 0.9);
            backdrop-filter: blur(20px);
        }

        .privacy-section h2 {
            text-align: center;
            font-size: 2.5rem;
            margin-bottom: 1rem;
            color: #f8fafc;
        }

        .privacy-intro {
            text-align: center;
            font-size: 1.1rem;
            margin-bottom: 3rem;
            color: #cbd5e1;
            max-width: 700px;
            margin-left: auto;
            margin-right: auto;
        }

        .privacy-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 2rem;
            margin-bottom: 2rem;
        }

        .privacy-item {
            display: flex;
            align-items: center;
            gap: 1rem;
            padding: 1.5rem;
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 10px;
            color: #cbd5e1;
            transition: all 0.3s ease;
        }

        .privacy-item:hover {
            transform: translateY(-3px);
            border-color: rgba(59, 130, 246, 0.4);
        }

        .privacy-icon {
            color: #10b981;
            font-size: 1.5rem;
            font-weight: bold;
        }

        .privacy-link {
            text-align: center;
            font-size: 1.1rem;
            color: #cbd5e1;
        }

        .link-blue {
            color: #3b82f6;
            text-decoration: none;
            border-bottom: 1px solid transparent;
            transition: border-color 0.2s;
        }

        .link-blue:hover {
            border-bottom-color: #3b82f6;
        }

        /* Footer */
        .footer {
            background: #0f1419;
            border-top: 1px solid rgba(59, 130, 246, 0.2);
            color: #cbd5e1;
            padding: 2rem 0;
        }

        .footer-content {
            display: flex;
            flex-direction: column;
            align-items: center;
            gap: 1rem;
        }

        .footer-links {
            display: flex;
            align-items: center;
            gap: 1rem;
        }

        .footer-links a {
            color: #3b82f6;
            text-decoration: none;
            transition: color 0.2s;
        }

        .footer-links a:hover {
            color: #60a5fa;
            text-decoration: underline;
        }

        .separator {
            color: rgba(59, 130, 246, 0.3);
        }

        /* Responsive */
        @media (max-width: 768px) {
            .hero h1 {
                font-size: 2.5rem;
            }
            
            .hero p {
                font-size: 1.1rem;
            }
            
            .hero-buttons {
                flex-direction: column;
                align-items: center;
            }
            
            .nav-links {
                display: none;
            }
            
            .auth-buttons {
                flex-direction: column;
                gap: 0.5rem;
            }

            .about-content {
                grid-template-columns: 1fr;
                gap: 2rem;
            }

            .about-stats {
                flex-direction: row;
                flex-wrap: wrap;
            }

            .stat-item {
                flex: 1;
                min-width: 150px;
            }

            .footer-content {
                text-align: center;
            }

            .footer-links {
                flex-wrap: wrap;
                justify-content: center;
            }
        }
    </style>
</head>
<body>
    <!-- Header -->
    <header class="header">
        <div class="container">
            <nav class="nav">
                <a href="/" class="logo">🎬 VideoSync</a>
                <div class="nav-links">
                    <a href="#workflow">Workflow</a>
                    <a href="/services">Services</a>
                    <a href="#pricing">Pricing</a>
                    <a href="#features">Features</a>
                </div>
                <div class="auth-buttons" id="homepageAuthButtons">
                    <a href="/login" class="btn btn-secondary">Login</a>
                    <a href="/signup" class="btn btn-primary">Sign Up</a>
                </div>
            </nav>
        </div>
    </header>

    <!-- Hero Section -->
    <section class="hero">
        <div class="container">
            <h1>AI Agent-Powered Video Production</h1>
            <p>The agent plans the work, generates animated scenes, edits footage, adds voiceovers, creates thumbnails, runs quality review, and publishes the result — all autonomously. Professional editing, 3D/2D animation, voiceover, captions, and delivery built into one agentic workflow.</p>
            <div style="background:rgba(122,76,255,0.15);border:1px solid rgba(122,76,255,0.4);border-radius:10px;padding:12px 18px;display:inline-block;margin:18px 0 14px;font-size:14px;color:#fff;font-weight:500">
                <strong>Edit clips</strong>, <strong>generate long-form videos</strong>, <strong>create thumbnails</strong>, and <strong>produce visuals</strong> from one chat-based workspace
            </div>
            <div class="hero-buttons" id="homepageHeroButtons">
                <a href="/signup" class="btn btn-primary btn-large">Start 7-Day Free Trial</a>
                <a href="#workflow" class="btn btn-secondary btn-large">See How It Works</a>
            </div>
            <div style="text-align:center;margin-top:30px">
                <a href="/services" class="btn btn-secondary">Explore Productized Services</a>
            </div>
        </div>
    </section>

    <!-- Workflow Section -->
    <section id="workflow" style="padding:60px 20px;background:#fff">
        <div class="container">
            <h2 style="text-align:center;margin-bottom:12px">One Chat, Full Video Production</h2>
            <p style="text-align:center;color:#666;margin-bottom:40px;max-width:760px;margin-left:auto;margin-right:auto">The $15/month creator plan is for the core VideoSync workspace: natural-language video editing and generation after a 7-day free trial. Productized client offers live on the Services page.</p>

            <div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:20px;max-width:1150px;margin:0 auto">
                <div style="background:#f8f9fa;border:1px solid #e5e7eb;border-radius:16px;padding:28px 24px">
                    <div style="font-size:12px;font-weight:700;letter-spacing:0.08em;color:#7a4cff;text-transform:uppercase;margin-bottom:8px">Natural Language</div>
                    <h3 style="margin:0 0 10px;color:#111">Tell the Agent What To Make</h3>
                    <p style="color:#666;font-size:14px;line-height:1.6;margin:12px 0 0">Ask for edits, new scenes, thumbnails, long-form explainers, voiceover, captions, or clips without learning a traditional editing timeline first.</p>
                </div>

                <div style="background:#f8f9fa;border:1px solid #e5e7eb;border-radius:16px;padding:28px 24px">
                    <div style="font-size:12px;font-weight:700;letter-spacing:0.08em;color:#7a4cff;text-transform:uppercase;margin-bottom:8px">Any Length</div>
                    <h3 style="margin:0 0 10px;color:#111">Shorts, Demos, or Long Videos</h3>
                    <p style="color:#666;font-size:14px;line-height:1.6;margin:12px 0 0">Generate quick clips, 30-60 second promos, multi-minute explainers, or longer structured videos by letting the workflow plan segments and assemble them.</p>
                </div>

                <div style="background:#f8f9fa;border:1px solid #e5e7eb;border-radius:16px;padding:28px 24px">
                    <div style="font-size:12px;font-weight:700;letter-spacing:0.08em;color:#7a4cff;text-transform:uppercase;margin-bottom:8px">Creative Tools</div>
                    <h3 style="margin:0 0 10px;color:#111">Editing + Generation Stack</h3>
                    <p style="color:#666;font-size:14px;line-height:1.6;margin:12px 0 0">Use professional editing tools, animated 3D/2D scenes, data visualizations, narration, thumbnails, stock support, and AI review from the same system.</p>
                </div>

                <div style="background:#f8f9fa;border:1px solid #e5e7eb;border-radius:16px;padding:28px 24px">
                    <div style="font-size:12px;font-weight:700;letter-spacing:0.08em;color:#7a4cff;text-transform:uppercase;margin-bottom:8px">Client Services</div>
                    <h3 style="margin:0 0 10px;color:#111">Need a Sellable Package?</h3>
                    <p style="color:#666;font-size:14px;line-height:1.6;margin:12px 0 0">SaaS launch packs, product mockups, clip packs, education videos, voice/audio work, and agency bundles are organized separately on the Services page.</p>
                </div>
            </div>
        </div>
    </section>

    <!-- Pricing Section -->
    <section id="pricing" style="padding:60px 20px;background:#f8f9fa">
        <div class="container">
            <h2 style="text-align:center;margin-bottom:12px">Start With the Core Creator Plan</h2>
            <p style="text-align:center;color:#666;margin-bottom:40px;max-width:760px;margin-left:auto;margin-right:auto">The homepage subscription is simple: 7 days free, then $15/month for the AI video editing and generation workspace. Higher-priced client service packages are listed on the Services page.</p>

            <div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:20px;max-width:1100px;margin:0 auto">

                <!-- Launch Packs -->
                <div style="background:#fff;border:1px solid #e5e7eb;border-radius:16px;padding:32px 26px;box-shadow:0 2px 10px rgba(0,0,0,0.04)">
                    <div style="font-size:12px;font-weight:700;letter-spacing:0.08em;color:#7a4cff;text-transform:uppercase;margin-bottom:6px">Launch Packs</div>
                    <div style="font-size:40px;font-weight:800;color:#111">$149<span style="font-size:15px;font-weight:500;color:#888;margin-left:6px">to $499</span></div>
                    <div style="color:#666;font-size:13px;margin:6px 0 16px">Best for SaaS founders and product launches</div>
                    <ul style="list-style:none;padding:0;margin:16px 0">
                        <li style="padding:7px 0;font-size:13.5px;color:#333;border-bottom:1px dashed #eee">✓ URL-to-video workflow for SaaS hero videos and demo reels</li>
                        <li style="padding:7px 0;font-size:13.5px;color:#333;border-bottom:1px dashed #eee">✓ 3D product mockups and launch visuals</li>
                        <li style="padding:7px 0;font-size:13.5px;color:#333;border-bottom:1px dashed #eee">✓ Delivery previews that close straight from a DM or landing page</li>
                        <li style="padding:7px 0;font-size:13.5px;color:#333;border-bottom:1px dashed #eee">✓ Optional upsell into launch bundles and cutdowns</li>
                        <li style="padding:7px 0;font-size:13.5px;color:#333">✓ Ideal when you want one strong asset fast</li>
                    </ul>
                    <a href="/signup" class="btn btn-primary" style="display:block;text-align:center;margin-top:12px">Create a sample →</a>
                </div>

                <!-- Creators -->
                <div style="background:#fff;border:2px solid #7a4cff;border-radius:16px;padding:32px 26px;box-shadow:0 8px 30px rgba(122,76,255,0.12);position:relative">
                    <div style="position:absolute;top:-12px;right:20px;background:#7a4cff;color:#fff;font-size:11px;font-weight:700;padding:5px 12px;border-radius:999px;letter-spacing:0.04em">BEST FOR RECURRING REVENUE</div>
                    <div style="font-size:12px;font-weight:700;letter-spacing:0.08em;color:#7a4cff;text-transform:uppercase;margin-bottom:6px">Creators</div>
                    <div style="font-size:40px;font-weight:800;color:#111">$15<span style="font-size:15px;font-weight:500;color:#888;margin-left:6px">/month</span></div>
                    <div style="color:#666;font-size:13px;margin:6px 0 16px">After 7-day free trial</div>
                    <ul style="list-style:none;padding:0;margin:16px 0">
                        <li style="padding:7px 0;font-size:13.5px;color:#333;border-bottom:1px dashed #eee">✓ AI thumbnails and creator growth assets</li>
                        <li style="padding:7px 0;font-size:13.5px;color:#333;border-bottom:1px dashed #eee">✓ Animated 3D/2D scenes, title cards, data visualizations, and UI mockups</li>
                        <li style="padding:7px 0;font-size:13.5px;color:#333;border-bottom:1px dashed #eee">✓ Professional editing engine for trims, effects, and delivery</li>
                        <li style="padding:7px 0;font-size:13.5px;color:#333;border-bottom:1px dashed #eee">✓ Delivery pages, previews, and portfolio samples</li>
                        <li style="padding:7px 0;font-size:13.5px;color:#333">✓ Best entry point for creators testing the workflow</li>
                    </ul>
                    <a href="/subscribe" class="btn btn-primary" style="display:block;text-align:center;margin-top:12px">Start 7-day trial →</a>
                </div>

                <!-- Agencies -->
                <div style="background:#fff;border:1px solid #e5e7eb;border-radius:16px;padding:32px 26px;box-shadow:0 2px 10px rgba(0,0,0,0.04)">
                    <div style="font-size:12px;font-weight:700;letter-spacing:0.08em;color:#7a4cff;text-transform:uppercase;margin-bottom:6px">Agencies</div>
                    <div style="font-size:40px;font-weight:800;color:#111">$99<span style="font-size:15px;font-weight:500;color:#888;margin-left:6px">to $199/mo</span></div>
                    <div style="color:#666;font-size:13px;margin:6px 0 16px">Starter or Pro tier - white-label API access</div>
                    <ul style="list-style:none;padding:0;margin:16px 0">
                        <li style="padding:7px 0;font-size:13.5px;color:#333;border-bottom:1px dashed #eee">✓ 1,000-5,000 clips / month</li>
                        <li style="padding:7px 0;font-size:13.5px;color:#333;border-bottom:1px dashed #eee">✓ 500-2,500 AI thumbnails / month</li>
                        <li style="padding:7px 0;font-size:13.5px;color:#333;border-bottom:1px dashed #eee">✓ Unlimited-length animated scenes (no 60-second cap)</li>
                        <li style="padding:7px 0;font-size:13.5px;color:#333;border-bottom:1px dashed #eee">✓ White-label /delivery/:id pages and launch-friendly outputs</li>
                        <li style="padding:7px 0;font-size:13.5px;color:#333">✓ API key + docs + agency-friendly resale economics</li>
                    </ul>
                    <a href="/api-access" class="btn btn-primary" style="display:block;text-align:center;margin-top:12px">See API Tiers →</a>
                </div>
            </div>

            <div style="text-align:center;margin-top:36px;padding:20px;background:#fff;border-radius:12px;max-width:740px;margin-left:auto;margin-right:auto;box-shadow:0 2px 10px rgba(0,0,0,0.04)">
                <div style="font-size:13px;font-weight:600;color:#7a4cff;letter-spacing:0.05em;text-transform:uppercase;margin-bottom:6px">How delivery monetization works</div>
                <p style="color:#666;font-size:14px;line-height:1.6;margin:0">You can sell directly from previews. Lightweight samples unlock from <strong>$19</strong>. Website-driven launch videos unlock from <strong>$197+</strong>. For recurring clients, move them into <strong>$15 creator access</strong> or the <strong>$99-$199 white-label API</strong>.</p>
            </div>
        </div>
    </section>

    <!-- Features Section -->
    <section class="features" id="features">
        <div class="container">
            <h2>Revolutionary Video Editing Experience</h2>
            <div class="features-grid">
                <div class="feature-card">
                    <div class="feature-icon">🤖</div>
                    <h3>AI-Powered Assistant</h3>
                    <p>Chat with our intelligent AI to edit videos using natural language. No need to learn complex tools or techniques.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">⚡</div>
                    <h3>Lightning Fast</h3>
                    <p>Process videos in seconds, not minutes. Our optimized backend handles complex operations efficiently.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">🎯</div>
                    <h3>Professional Quality</h3>
                    <p>Get broadcast-quality results with advanced algorithms and professional-grade video processing.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">🎥</div>
                    <h3>YouTube Integration</h3>
                    <p>Upload directly to YouTube, manage videos, track analytics, optimize metadata, and moderate comments—all from one place.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">🔒</div>
                    <h3>Secure & Private</h3>
                    <p>Your videos are processed securely with enterprise-grade encryption and privacy protection.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">💾</div>
                    <h3>Smart Memory</h3>
                    <p>Our AI remembers your preferences and past projects to provide better, more personalized results.</p>
                </div>
            </div>
        </div>
    </section>

    <!-- Tools Section -->
    <section class="tools" id="tools">
        <div class="container">
            <h2>Professional Video Editing & Generation Engine</h2>
            <p style="text-align:center;color:#666;margin-bottom:2rem;">Every tool is available via natural language. Just describe what you want. Videos of any length — no 60-second cap.</p>
            <div class="tools-grid">
                <div class="tool-category">
                    <h3>🎬 Core Editing</h3>
                    <ul class="tool-list">
                        <li>Trim, Cut, Merge, Split</li>
                        <li>Deshake / Stabilize (vid.stab)</li>
                        <li>Reverse, Loop, Concatenate</li>
                        <li>Scene Detection & Analysis</li>
                        <li>Segment & Chapter Split</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>🎨 Visual Effects (100+)</h3>
                    <ul class="tool-list">
                        <li>Color Grading & LUT3D</li>
                        <li>Cinematic Film Grain & Vignette</li>
                        <li>Chroma Key / Green Screen</li>
                        <li>Motion Blur, Glow, Bloom</li>
                        <li>Edge Detect, Posterize, Solarize</li>
                        <li>Vintage Curves, Vibrance, HSV</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>🔊 Audio Processing (80+)</h3>
                    <ul class="tool-list">
                        <li>Audio normalization (EBU R128 / LUFS)</li>
                        <li>RNN Denoise & De-esser</li>
                        <li>Equalizer, Compressor, Limiter</li>
                        <li>CQT & Spectrum Visualization</li>
                        <li>Stereo Widener, Surround Mix</li>
                        <li>Pitch Shift, Time Stretch</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>📊 Analysis & Review (40+)</h3>
                    <ul class="tool-list">
                        <li>Quality metrics (VMAF/SSIM/PSNR)</li>
                        <li>Loudness & Silence Detection</li>
                        <li>Scene Change Detection</li>
                        <li>Black Frame & Freeze Detect</li>
                        <li>Bitrate & Metadata Extraction</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>📤 Platform Export</h3>
                    <ul class="tool-list">
                        <li>YouTube / TikTok / Instagram</li>
                        <li>Format Conversion (20+ formats)</li>
                        <li>H.264 / H.265 / VP9 / AV1</li>
                        <li>GIF with Palette Optimization</li>
                        <li>HDR to SDR Tone Mapping</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>⚡ Workflow Recipes</h3>
                    <ul class="tool-list">
                        <li>YouTube-Ready Export</li>
                        <li>Podcast Audio Cleanup</li>
                        <li>Cinematic Grade</li>
                        <li>Talking Head Cleanup</li>
                        <li>GIF Creator</li>
                    </ul>
                </div>
            </div>
        </div>
    </section>

    <!-- About Section -->
    <section class="about" id="about">
        <div class="container">
            <h2>About VideoSync</h2>
            <div class="about-content">
                <div class="about-text">
                    <h3>Revolutionizing Video Editing with AI</h3>
                    <p>Our AI-powered video editor transforms the way creators work with video content. Instead of learning complex software interfaces, simply describe what you want in natural language, and our intelligent system handles the technical details.</p>
                    
                    <h3>Built for Modern Creators</h3>
                    <p>Whether you're a content creator, marketer, educator, or filmmaker, our platform adapts to your workflow. From simple cuts and transitions to advanced effects and color grading, experience professional-quality results without the learning curve.</p>
                    
                    <h3>Secure & Reliable</h3>
                    <p>Your content is processed with enterprise-grade security and privacy protection. All video processing happens in our secure cloud infrastructure, ensuring your creative work remains safe and confidential.</p>
                </div>
                <div class="about-stats">
                    <div class="stat-item">
                        <div class="stat-number">10,000+</div>
                        <div class="stat-label">Videos Processed</div>
                    </div>
                    <div class="stat-item">
                        <div class="stat-number">2,500+</div>
                        <div class="stat-label">Active Users</div>
                    </div>
                    <div class="stat-item">
                        <div class="stat-number">99.9%</div>
                        <div class="stat-label">Uptime</div>
                    </div>
                </div>
            </div>
        </div>
    </section>

    <!-- Privacy & Security Section -->
    <section class="privacy-section" id="privacy">
        <div class="container">
            <h2>🔒 Your Data, Your Control</h2>
            <p class="privacy-intro">We prioritize your privacy and security. Our platform is built with transparency and compliance at its core.</p>
            <div class="privacy-grid">
                <div class="privacy-item">
                    <div class="privacy-icon">✓</div>
                    <div>Encrypted OAuth tokens</div>
                </div>
                <div class="privacy-item">
                    <div class="privacy-icon">✓</div>
                    <div>GDPR & CCPA compliant</div>
                </div>
                <div class="privacy-item">
                    <div class="privacy-icon">✓</div>
                    <div>YouTube API Terms compliant</div>
                </div>
                <div class="privacy-item">
                    <div class="privacy-icon">✓</div>
                    <div>Revoke access anytime</div>
                </div>
            </div>
            <p class="privacy-link">Learn more in our <a href="/privacy" class="link-blue">Privacy Policy</a> and <a href="/terms" class="link-blue">Terms of Service</a>.</p>
        </div>
    </section>

    <!-- CTA Section -->
    <section class="cta">
        <div class="container">
            <h2>Ready to Transform Your Videos?</h2>
            <p>Join thousands of creators who are already using AI to create amazing videos effortlessly.</p>
            <a href="/signup" class="btn btn-primary btn-large">Start Creating Now</a>
        </div>
    </section>

    <!-- Footer -->
    <footer class="footer">
        <div class="container">
            <div class="footer-content">
                <p>&copy; 2025 VideoSync. Professional AI-powered video editing solutions.</p>
                <div class="footer-links">
                    <a href="/privacy">Privacy Policy</a>
                    <span class="separator">|</span>
                    <a href="/terms">Terms of Service</a>
                    <span class="separator">|</span>
                    <a href="/help">Help & Support</a>
                </div>
            </div>
        </div>
    </footer>

    <script>
        class DynamicBackgroundManager {
            constructor() {
                this.lastBackgroundUpdate = Date.now();
                this.updateInterval = 5 * 60 * 1000; // 5 minutes
                this.retryDelay = 30 * 1000; // 30 seconds on error
                this.isUpdating = false;
                
                this.init();
            }

            async init() {
                // Load initial background
                await this.updateBackground();
                
                // Set up periodic updates
                setInterval(() => {
                    this.checkAndUpdateBackground();
                }, 60 * 1000); // Check every minute
            }

            async checkAndUpdateBackground() {
                if (this.isUpdating) return;
                
                const timeSinceLastUpdate = Date.now() - this.lastBackgroundUpdate;
                if (timeSinceLastUpdate >= this.updateInterval) {
                    await this.updateBackground();
                }
            }

            async updateBackground() {
                if (this.isUpdating) return;
                
                this.isUpdating = true;
                
                try {
                    console.log('🎨 Fetching new dynamic background...');
                    
                    const response = await fetch('/api/background/image');
                    
                    if (response.ok) {
                        const contentType = response.headers.get('content-type');
                        
                        if (contentType && contentType.includes('application/json')) {
                            // Fallback gradient
                            const data = await response.json();
                            if (data.fallback && data.gradient) {
                                document.body.style.background = data.gradient;
                                console.log('🎨 Applied fallback gradient background');
                            }
                        } else {
                            // Image response
                            const blob = await response.blob();
                            const imageUrl = URL.createObjectURL(blob);
                            
                            // Create overlay for smooth transition
                            const overlay = document.createElement('div');
                            overlay.style.cssText = `
                                position: fixed;
                                top: 0;
                                left: 0;
                                width: 100%;
                                height: 100%;
                                background-image: url(${imageUrl});
                                background-size: cover;
                                background-position: center;
                                background-attachment: fixed;
                                opacity: 0;
                                transition: opacity 1s ease-in-out;
                                z-index: -1;
                                pointer-events: none;
                            `;
                            
                            document.body.appendChild(overlay);
                            
                            // Trigger fade in
                            setTimeout(() => {
                                overlay.style.opacity = '0.3'; // Semi-transparent overlay
                            }, 100);
                            
                            // Clean up old overlays after transition
                            setTimeout(() => {
                                const oldOverlays = document.querySelectorAll('div[style*="background-image"]');
                                oldOverlays.forEach((old, index) => {
                                    if (index < oldOverlays.length - 1) {
                                        old.remove();
                                    }
                                });
                            }, 1100);
                            
                            console.log('🎨 Applied new AI-generated background');
                        }
                        
                        this.lastBackgroundUpdate = Date.now();
                    } else {
                        console.warn('Failed to fetch background image:', response.status);
                    }
                } catch (error) {
                    console.error('Error updating background:', error);
                    // Retry with shorter delay on error
                    setTimeout(() => {
                        this.lastBackgroundUpdate = Date.now() - this.updateInterval + this.retryDelay;
                    }, this.retryDelay);
                } finally {
                    this.isUpdating = false;
                }
            }
        }

        function getStoredHomepageToken() {
            return localStorage.getItem('auth_token')
                || localStorage.getItem('authToken')
                || localStorage.getItem('admin_token')
                || '';
        }

        function swapHomepageCtasForAuthenticatedUser() {
            const authToken = getStoredHomepageToken();
            if (!authToken) return;

            const navButtons = document.getElementById('homepageAuthButtons');
            const heroButtons = document.getElementById('homepageHeroButtons');

            if (navButtons) {
                navButtons.innerHTML = [
                    '<a href="/dashboard" class="btn btn-primary">Dashboard</a>',
                    '<a href="/chat" class="btn btn-secondary">Open Chat</a>'
                ].join('');
            }

            if (heroButtons) {
                heroButtons.innerHTML = [
                    '<a href="/chat" class="btn btn-primary btn-large">Start New Chat</a>',
                    '<a href="/dashboard" class="btn btn-secondary btn-large">Go to Dashboard</a>'
                ].join('');
            }
        }

        // Initialize interactive background effects without blocking the auth-state UI swap.
        document.addEventListener('DOMContentLoaded', () => {
            swapHomepageCtasForAuthenticatedUser();

            try {
                new DynamicBackgroundManager();
            } catch (error) {
                console.error('Dynamic background initialization failed:', error);
            }
        });

        // Run once immediately as well, since this script is already at the end of <body>.
        swapHomepageCtasForAuthenticatedUser();

        // Add subtle loading indicator for background updates
        let backgroundLoadingIndicator = null;

        function showBackgroundLoading() {
            if (backgroundLoadingIndicator) return;
            
            backgroundLoadingIndicator = document.createElement('div');
            backgroundLoadingIndicator.innerHTML = '🎨 Refreshing background...';
            backgroundLoadingIndicator.style.cssText = `
                position: fixed;
                top: 20px;
                right: 20px;
                background: rgba(0, 0, 0, 0.8);
                color: white;
                padding: 8px 16px;
                border-radius: 20px;
                font-size: 12px;
                z-index: 1000;
                opacity: 0;
                transition: opacity 0.3s ease;
            `;
            backgroundLoadingIndicator.textContent = 'Refreshing background...';
            
            document.body.appendChild(backgroundLoadingIndicator);
            setTimeout(() => {
                backgroundLoadingIndicator.style.opacity = '1';
            }, 100);
            
            setTimeout(() => {
                if (backgroundLoadingIndicator) {
                    backgroundLoadingIndicator.style.opacity = '0';
                    setTimeout(() => {
                        if (backgroundLoadingIndicator) {
                            backgroundLoadingIndicator.remove();
                            backgroundLoadingIndicator = null;
                        }
                    }, 300);
                }
            }, 3000);
        }
    </script>
</body>
</html>
    "###
}

fn build_modern_landing_page_html() -> &'static str {
    r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>VideoSync - AI Agent-Powered Video Production</title>
    <style>
        :root {
            --bg: #07111d;
            --panel: rgba(9, 18, 31, 0.82);
            --panel-strong: rgba(7, 14, 24, 0.92);
            --line: rgba(148, 163, 184, 0.16);
            --line-strong: rgba(96, 165, 250, 0.28);
            --text: #e5eefb;
            --muted: #a8b8d3;
            --blue: #60a5fa;
            --blue-strong: #3b82f6;
            --green: #22c55e;
            --shadow: 0 24px 70px rgba(2, 6, 23, 0.45);
        }

        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        html {
            scroll-behavior: smooth;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
            line-height: 1.6;
            color: var(--text);
            background:
                radial-gradient(circle at top left, rgba(59, 130, 246, 0.18), transparent 28%),
                linear-gradient(135deg, #0a1322 0%, #0d1728 55%, #07111d 100%);
            background-size: cover;
            background-position: center;
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
        }

        a {
            color: inherit;
        }

        .container {
            max-width: 1200px;
            margin: 0 auto;
            padding: 0 20px;
        }

        .header {
            position: fixed;
            top: 0;
            width: 100%;
            z-index: 1000;
            background: rgba(4, 10, 18, 0.78);
            backdrop-filter: blur(18px);
            border-bottom: 1px solid rgba(96, 165, 250, 0.16);
        }

        .nav {
            min-height: 78px;
            display: flex;
            justify-content: space-between;
            align-items: center;
            gap: 1rem;
        }

        .logo {
            font-size: 1.45rem;
            font-weight: 800;
            text-decoration: none;
            letter-spacing: 0.02em;
        }

        .nav-links,
        .auth-buttons {
            display: flex;
            align-items: center;
            gap: 1rem;
        }

        .nav-links a {
            text-decoration: none;
            color: #dbeafe;
            padding: 0.55rem 0.95rem;
            border-radius: 999px;
            transition: background-color 0.25s ease;
        }

        .nav-links a:hover {
            background: rgba(59, 130, 246, 0.14);
        }

        .btn {
            display: inline-flex;
            align-items: center;
            justify-content: center;
            padding: 0.8rem 1.35rem;
            border-radius: 999px;
            text-decoration: none;
            font-weight: 700;
            transition: transform 0.25s ease, box-shadow 0.25s ease, background 0.25s ease;
        }

        .btn:hover {
            transform: translateY(-1px);
        }

        .btn-primary {
            background: linear-gradient(135deg, #3b82f6, #2563eb);
            color: white;
            box-shadow: 0 14px 35px rgba(37, 99, 235, 0.28);
        }

        .btn-secondary {
            background: rgba(15, 23, 42, 0.7);
            border: 1px solid rgba(96, 165, 250, 0.22);
            color: #dbeafe;
        }

        .btn-large {
            padding: 1rem 1.8rem;
            font-size: 1rem;
        }

        .hero {
            padding: 126px 0 72px;
        }

        .hero-badge {
            display: inline-flex;
            align-items: center;
            gap: 0.7rem;
            padding: 0.85rem 1.2rem;
            border-radius: 999px;
            border: 1px solid var(--line-strong);
            background: rgba(8, 15, 28, 0.78);
            color: #dbeafe;
            font-size: 0.95rem;
            box-shadow: var(--shadow);
            opacity: 0;
            transform: translateY(24px);
            animation: fadeInUp 0.8s ease-out 0.1s forwards;
        }

        .hero-badge svg,
        .feature-icon svg,
        .tool-heading-icon svg {
            width: 28px;
            height: 28px;
            stroke: var(--blue);
        }

        .hero h1 {
            margin-top: 1.25rem;
            font-size: 3.6rem;
            line-height: 1.05;
            max-width: 980px;
            opacity: 0;
            transform: translateY(28px);
            animation: fadeInUp 0.85s ease-out 0.2s forwards;
        }

        .hero-copy {
            margin-top: 1.3rem;
            max-width: 840px;
            font-size: 1.18rem;
            color: var(--muted);
            opacity: 0;
            transform: translateY(28px);
            animation: fadeInUp 0.85s ease-out 0.35s forwards;
        }

        .hero-buttons {
            display: flex;
            flex-wrap: wrap;
            gap: 1rem;
            margin-top: 2rem;
            opacity: 0;
            transform: translateY(28px);
            animation: fadeInUp 0.85s ease-out 0.5s forwards;
        }

        .hero-showcase {
            margin-top: 2.2rem;
            display: grid;
            grid-template-columns: minmax(0, 1.15fr) minmax(300px, 0.85fr);
            gap: 1.4rem;
            opacity: 0;
            transform: translateY(28px);
            animation: fadeInUp 0.9s ease-out 0.65s forwards;
        }

        .hero-slider,
        .hero-summary {
            border-radius: 28px;
            border: 1px solid var(--line);
            background: var(--panel);
            box-shadow: var(--shadow);
            backdrop-filter: blur(18px);
        }

        .hero-slider {
            position: relative;
            overflow: hidden;
            min-height: 300px;
            background:
                radial-gradient(circle at top left, rgba(59, 130, 246, 0.28), transparent 34%),
                radial-gradient(circle at bottom right, rgba(34, 197, 94, 0.15), transparent 28%),
                var(--panel-strong);
        }

        .hero-slide {
            position: absolute;
            inset: 0;
            padding: 2rem;
            display: flex;
            flex-direction: column;
            justify-content: space-between;
            opacity: 0;
            transform: translateY(14px);
            animation: heroCycle 20s infinite;
        }

        .hero-slide:nth-child(1) { animation-delay: 0s; }
        .hero-slide:nth-child(2) { animation-delay: 5s; }
        .hero-slide:nth-child(3) { animation-delay: 10s; }
        .hero-slide:nth-child(4) { animation-delay: 15s; }

        .hero-slide-tag {
            display: inline-flex;
            width: fit-content;
            padding: 0.45rem 0.8rem;
            border-radius: 999px;
            font-size: 0.78rem;
            font-weight: 800;
            letter-spacing: 0.08em;
            text-transform: uppercase;
            color: #dbeafe;
            background: rgba(59, 130, 246, 0.14);
            border: 1px solid rgba(96, 165, 250, 0.2);
        }

        .hero-slide h3 {
            margin-top: 1rem;
            font-size: 1.95rem;
            line-height: 1.1;
        }

        .hero-slide p {
            margin-top: 0.9rem;
            max-width: 680px;
            color: #c9d7ef;
        }

        .hero-slide-points {
            display: flex;
            flex-wrap: wrap;
            gap: 0.65rem;
            margin-top: 1.2rem;
        }

        .hero-slide-points span {
            padding: 0.48rem 0.78rem;
            border-radius: 999px;
            background: rgba(15, 23, 42, 0.7);
            border: 1px solid rgba(148, 163, 184, 0.18);
            color: #e2e8f0;
            font-size: 0.88rem;
        }

        .hero-summary {
            padding: 1.6rem;
            display: flex;
            flex-direction: column;
            gap: 1.25rem;
        }

        .hero-summary-label,
        .section-kicker {
            font-size: 0.8rem;
            letter-spacing: 0.08em;
            text-transform: uppercase;
            color: #93c5fd;
            font-weight: 800;
        }

        .hero-summary h3 {
            font-size: 1.6rem;
            line-height: 1.15;
        }

        .hero-summary p {
            color: var(--muted);
        }

        .hero-metrics {
            display: grid;
            grid-template-columns: repeat(2, minmax(0, 1fr));
            gap: 0.85rem;
        }

        .hero-metric {
            padding: 1rem;
            border-radius: 18px;
            background: rgba(15, 23, 42, 0.72);
            border: 1px solid rgba(148, 163, 184, 0.14);
        }

        .hero-metric strong {
            display: block;
            font-size: 1.15rem;
        }

        .hero-metric span {
            display: block;
            margin-top: 0.2rem;
            color: #94a3b8;
            font-size: 0.84rem;
        }

        .section {
            padding: 74px 0;
        }

        .section-dark {
            background: rgba(6, 12, 20, 0.58);
            backdrop-filter: blur(16px);
        }

        .section-intro {
            text-align: center;
            max-width: 820px;
            margin: 0 auto 2.6rem;
        }

        .section-intro h2 {
            margin-top: 0.6rem;
            font-size: 2.55rem;
        }

        .section-intro p {
            margin-top: 0.75rem;
            color: var(--muted);
        }

        .offer-grid,
        .pricing-grid,
        .features-grid,
        .tools-grid {
            display: grid;
            gap: 1.3rem;
        }

        .offer-grid {
            grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
        }

        .pricing-grid {
            grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
        }

        .features-grid {
            grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
        }

        .tools-grid {
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
        }

        .offer-card,
        .pricing-card,
        .feature-card,
        .tool-category,
        .about-card,
        .cta-card {
            border-radius: 22px;
            border: 1px solid var(--line);
            background: var(--panel);
            box-shadow: var(--shadow);
            backdrop-filter: blur(16px);
        }

        .offer-card,
        .pricing-card,
        .tool-category,
        .about-card {
            padding: 1.7rem;
        }

        .offer-card h3,
        .pricing-card h3 {
            font-size: 1.45rem;
            margin-top: 0.55rem;
        }

        .offer-card p,
        .pricing-card p {
            color: var(--muted);
        }

        .eyebrow {
            color: #93c5fd;
            font-size: 0.8rem;
            letter-spacing: 0.08em;
            text-transform: uppercase;
            font-weight: 800;
        }

        .price {
            margin-top: 0.75rem;
            font-size: 2rem;
            font-weight: 800;
        }

        .pricing-card.featured {
            border: 2px solid rgba(96, 165, 250, 0.58);
            position: relative;
        }

        .featured-badge {
            position: absolute;
            top: -12px;
            right: 18px;
            padding: 0.35rem 0.75rem;
            border-radius: 999px;
            background: var(--blue-strong);
            color: white;
            font-size: 0.72rem;
            font-weight: 800;
            letter-spacing: 0.05em;
            text-transform: uppercase;
        }

        .pricing-list,
        .tool-list {
            list-style: none;
            margin-top: 1rem;
        }

        .pricing-list li,
        .tool-list li {
            position: relative;
            padding: 0.45rem 0 0.45rem 1rem;
            color: #d7e3f5;
        }

        .pricing-list li::before,
        .tool-list li::before {
            content: "";
            position: absolute;
            left: 0;
            top: 0.95rem;
            width: 6px;
            height: 6px;
            border-radius: 999px;
            background: var(--green);
        }

        .pricing-note {
            max-width: 760px;
            margin: 2rem auto 0;
            padding: 1.2rem 1.3rem;
            border-radius: 18px;
            border: 1px solid var(--line);
            background: rgba(9, 18, 31, 0.82);
            color: var(--muted);
            text-align: center;
            box-shadow: var(--shadow);
        }

        .feature-card {
            padding: 2rem;
            text-align: center;
        }

        .feature-icon {
            width: 74px;
            height: 74px;
            margin: 0 auto 1rem;
            display: inline-flex;
            align-items: center;
            justify-content: center;
            border-radius: 22px;
            background: linear-gradient(135deg, rgba(59, 130, 246, 0.22), rgba(14, 165, 233, 0.06));
            border: 1px solid rgba(96, 165, 250, 0.18);
        }

        .feature-card h3 {
            font-size: 1.35rem;
        }

        .feature-card p {
            margin-top: 0.8rem;
            color: var(--muted);
        }

        .tool-category h3 {
            display: flex;
            align-items: center;
            gap: 0.7rem;
            color: #dbeafe;
            font-size: 1.2rem;
        }

        .tool-heading-icon {
            width: 42px;
            height: 42px;
            display: inline-flex;
            align-items: center;
            justify-content: center;
            border-radius: 14px;
            background: rgba(59, 130, 246, 0.12);
            border: 1px solid rgba(96, 165, 250, 0.18);
            flex-shrink: 0;
        }

        .tools-subcopy {
            margin: -1rem auto 2.2rem;
            max-width: 760px;
            text-align: center;
            color: var(--muted);
        }

        .about-grid {
            display: grid;
            grid-template-columns: minmax(0, 1.25fr) minmax(260px, 0.75fr);
            gap: 1.3rem;
        }

        .about-card h3 {
            margin-bottom: 0.75rem;
            color: #dbeafe;
        }

        .about-card p + h3 {
            margin-top: 1.35rem;
        }

        .about-card p {
            color: var(--muted);
        }

        .stat-stack {
            display: grid;
            gap: 1rem;
        }

        .stat-item {
            padding: 1.15rem;
            border-radius: 18px;
            background: rgba(15, 23, 42, 0.72);
            border: 1px solid rgba(148, 163, 184, 0.14);
        }

        .stat-item strong {
            display: block;
            font-size: 2rem;
            color: #f8fafc;
        }

        .stat-item span {
            color: var(--muted);
        }

        .cta-card {
            padding: 2.1rem;
            text-align: center;
            background:
                radial-gradient(circle at top left, rgba(59, 130, 246, 0.24), transparent 28%),
                rgba(8, 15, 28, 0.86);
        }

        .cta-card h2 {
            font-size: 2.4rem;
        }

        .cta-card p {
            max-width: 700px;
            margin: 0.9rem auto 1.8rem;
            color: var(--muted);
        }

        .footer {
            padding: 2rem 0 2.5rem;
        }

        .footer-content {
            display: flex;
            justify-content: space-between;
            align-items: center;
            gap: 1rem;
            flex-wrap: wrap;
            color: #94a3b8;
        }

        .footer-links {
            display: flex;
            align-items: center;
            gap: 1rem;
            flex-wrap: wrap;
        }

        .footer-links a {
            text-decoration: none;
            color: #93c5fd;
        }

        .footer-links span {
            color: rgba(148, 163, 184, 0.35);
        }

        @keyframes fadeInUp {
            from {
                opacity: 0;
                transform: translateY(28px);
            }
            to {
                opacity: 1;
                transform: translateY(0);
            }
        }

        @keyframes heroCycle {
            0% { opacity: 0; transform: translateY(14px); }
            4% { opacity: 1; transform: translateY(0); }
            21% { opacity: 1; transform: translateY(0); }
            25% { opacity: 0; transform: translateY(-10px); }
            100% { opacity: 0; transform: translateY(-10px); }
        }

        @media (max-width: 900px) {
            .hero-showcase,
            .about-grid {
                grid-template-columns: 1fr;
            }

            .hero h1 {
                font-size: 2.9rem;
            }
        }

        @media (max-width: 768px) {
            .nav-links {
                display: none;
            }

            .auth-buttons {
                flex-direction: column;
                gap: 0.55rem;
            }

            .hero {
                padding-top: 112px;
            }

            .hero h1 {
                font-size: 2.45rem;
            }

            .hero-buttons {
                flex-direction: column;
                align-items: stretch;
            }

            .hero-slider {
                min-height: 360px;
            }

            .hero-metrics {
                grid-template-columns: 1fr;
            }

            .section-intro h2,
            .cta-card h2 {
                font-size: 2rem;
            }

            .footer-content {
                justify-content: center;
                text-align: center;
            }
        }
    </style>
</head>
<body>
    <header class="header">
        <div class="container">
            <nav class="nav">
                <a href="/" class="logo">VideoSync</a>
                <div class="nav-links">
                    <a href="#workflow">Workflow</a>
                    <a href="#pricing">Plans</a>
                    <a href="/services">Services</a>
                    <a href="#features">Features</a>
                    <a href="#tools">Toolkit</a>
                </div>
                <div class="auth-buttons" id="homepageAuthButtons">
                    <a href="/login" class="btn btn-secondary">Login</a>
                    <a href="/signup" class="btn btn-primary">Sign Up</a>
                </div>
            </nav>
        </div>
    </header>

    <section class="hero">
        <div class="container">
            <div class="hero-badge">
                <svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M12 3l7 4v10l-7 4-7-4V7l7-4Z"></path>
                    <path d="m8.5 12 2.2 2.2 4.8-4.8"></path>
                </svg>
                <span>Agent-driven video editing and production from natural language</span>
            </div>
            <h1>Generate and Edit Videos of Any Length With AI</h1>
            <p class="hero-copy">VideoSync is an <strong>AI-powered video editing and generation workspace</strong>. Tell the agent what you want in natural language: edit clips, create thumbnails, generate short promos, build long-form videos, add voice, render animations, and package the result for download or delivery.</p>
            <div class="hero-buttons" id="homepageHeroButtons">
                <a href="/subscribe" class="btn btn-primary btn-large">Start 7-day trial</a>
                <a href="#pricing" class="btn btn-secondary btn-large">See plans</a>
            </div>
            <div class="hero-showcase" id="workflow">
                <div class="hero-slider">
                    <article class="hero-slide">
                        <div>
                            <span class="hero-slide-tag">Core Workspace</span>
                            <h3>The $15/month plan covers the AI editor and generator</h3>
                            <p>Natural-language editing, video generation, thumbnails, voice, stock footage, animated 3D/2D scenes, animated math/science visuals, and delivery/download workflows in one creator workspace.</p>
                        </div>
                        <div class="hero-slide-points">
                            <span>Full editing capabilities</span>
                            <span>Pexels footage</span>
                            <span>ElevenLabs voice</span>
                            <span>Animated 3D/2D scenes</span>
                        </div>
                    </article>
                    <article class="hero-slide">
                        <div>
                            <span class="hero-slide-tag">Any Length</span>
                            <h3>Generate quick clips or longer structured videos</h3>
                            <p>Ask for a 30-second promo, a 10-minute explainer, or a longer segmented production. The workflow can plan scenes, generate assets, assemble segments, and review the output.</p>
                        </div>
                        <div class="hero-slide-points">
                            <span>Short clips</span>
                            <span>Long-form videos</span>
                            <span>Segment planning</span>
                            <span>QA review</span>
                        </div>
                    </article>
                    <article class="hero-slide">
                        <div>
                            <span class="hero-slide-tag">Editing Tools</span>
                            <h3>Use the creative stack without learning every tool</h3>
                            <p>The agent can use professional editing, thumbnails, voice, stock footage, animated 3D/2D scenes, data visualizations, and review tools depending on what the video needs.</p>
                        </div>
                        <div class="hero-slide-points">
                            <span>Thumbnails</span>
                            <span>Professional edits</span>
                            <span>Voice/audio</span>
                            <span>Cross-platform exports</span>
                        </div>
                    </article>
                    <article class="hero-slide">
                        <div>
                            <span class="hero-slide-tag">Services Page</span>
                            <h3>Client packages live separately from the core product</h3>
                            <p>SaaS packs, mockups, education videos, clip packs, voice/audio work, and agency bundles are productized on the Services page so the homepage stays simple.</p>
                        </div>
                        <div class="hero-slide-points">
                            <span>SaaS packs</span>
                            <span>Mockups</span>
                            <span>Education</span>
                            <span>Agency bundles</span>
                        </div>
                    </article>
                </div>
                <aside class="hero-summary">
                    <div class="hero-summary-label">Two ways to work with us</div>
                    <h3>Subscribe to the AI workspace or order a done-for-you video.</h3>
                    <p>Use the $15/mo workspace to edit and generate yourself, or let our AI agent produce a custom video for you — delivered in hours, priced at a fraction of agency cost.</p>
                    <div class="hero-metrics">
                        <div class="hero-metric">
                            <strong>$15/mo</strong>
                            <span>AI Video Workspace — do it yourself</span>
                        </div>
                        <div class="hero-metric">
                            <strong>$149-$2,500</strong>
                            <span>Done-For-You — we produce it for you</span>
                        </div>
                    </div>
                </aside>
            </div>
        </div>
    </section>

    <section id="pricing" class="section">
        <div class="container">
            <div class="section-intro">
                <div class="section-kicker">Two Tiers, No Confusion</div>
                <h2>DIY Workspace or Done-For-You Production</h2>
                <p>Choose the <strong>$15/month workspace</strong> to edit and generate yourself, or order a <strong>custom video production</strong> starting at $149 — our AI agent does the work and delivers in hours, not weeks.</p>
            </div>
            <div class="pricing-grid">
                <article class="pricing-card featured">
                    <div class="featured-badge">Tier 1 — DIY</div>
                    <div class="eyebrow">AI Video Workspace</div>
                    <h3>$15<span style="font-size:0.5em;color:#94a3b8;margin-left:0.35rem">/month</span></h3>
                    <p>7-day free trial. Natural-language editing, video generation, animated 3D/2D scenes, animated explainers, voice, thumbnails, and delivery pages — all from one workspace. Videos of any length supported — no 60-second cap.</p>
                    <ul class="pricing-list">
                        <li>AI editing, generation, and creator growth assets</li>
                        <li>Animated 3D/2D scenes, animated explainers, UI mockups, and title cards</li>
                        <li>Professional video trims, exports, and delivery handoff</li>
                        <li>Pexels footage, ElevenLabs voice, previews, and delivery pages</li>
                        <li>Best entry point for recurring revenue</li>
                    </ul>
                    <a href="/subscribe" class="btn btn-primary" style="margin-top:1rem">Start 7-day trial</a>
                </article>
                <article class="pricing-card">
                    <div class="featured-badge" style="background:var(--accent-secondary,#f59e0b)">Tier 2 — DFY</div>
                    <div class="eyebrow">Done-For-You Production</div>
                    <h3>$149<span style="font-size:0.5em;color:#94a3b8;margin-left:0.35rem">to $2,500</span></h3>
                    <p>One-off project pricing. SaaS hero videos, product demos, 3D mockups, animated scenes, explainers, audiograms, clip packs, and white-label production — priced at a fraction of agency cost because our AI agent does the heavy lifting.</p>
                    <ul class="pricing-list">
                        <li>SaaS hero videos and website demos from a live URL</li>
                        <li>3D product mockups, cinematic loops, and launch visuals</li>
                        <li>Clip packs, thumbnails, audiograms, and short-form cutdowns</li>
                        <li>Full animated 3D/2D scenes and animated explainer videos</li>
                        <li>White-label delivery pages with preview links you can sell</li>
                    </ul>
                    <a href="/services" class="btn btn-secondary" style="margin-top:1rem">See All Services & Prices</a>
                </article>
            </div>
        </div>
    </section>

    <section id="features" class="section">
        <div class="container">
            <div class="section-intro">
                <div class="section-kicker">Platform Strengths</div>
                <h2>AI video editing stays central to the whole business</h2>
                <p>The premium offers matter, but they work because the core platform can already edit, generate, package, and deliver a wide range of outputs from one agentic workflow.</p>
            </div>
            <div class="features-grid">
                <article class="feature-card">
                    <div class="feature-icon">
                        <svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                            <rect x="7" y="4" width="10" height="13" rx="3"></rect>
                            <path d="M10 2v2M14 2v2M9 11h6M12 8v6M9 20h6"></path>
                        </svg>
                    </div>
                    <h3>AI-Powered Assistant</h3>
                    <p>Chat with the agent to edit footage, generate scenes, produce voice, build animations, and ship exports without learning a complex editor first.</p>
                </article>
                <article class="feature-card">
                    <div class="feature-icon">
                        <svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M13 2 4 14h6l-1 8 9-12h-6l1-8Z"></path>
                        </svg>
                    </div>
                    <h3>Fast Iteration</h3>
                    <p>Previews, delivery pages, and repeatable recipes make it easier to move from concept to monetizable output quickly.</p>
                </article>
                <article class="feature-card">
                    <div class="feature-icon">
                        <svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                            <circle cx="12" cy="12" r="8"></circle>
                            <circle cx="12" cy="12" r="4"></circle>
                            <path d="M12 8v8M8 12h8"></path>
                        </svg>
                    </div>
                    <h3>Professional Quality</h3>
                    <p>Blend professional editing with animated motion design, animated explainers, and polished handoff for outputs that feel client-ready.</p>
                </article>
                <article class="feature-card">
                    <div class="feature-icon">
                        <svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                            <rect x="3" y="6" width="15" height="12" rx="2"></rect>
                            <path d="m18 10 3-2v8l-3-2"></path>
                        </svg>
                    </div>
                    <h3>YouTube Integration</h3>
                    <p>Upload directly to YouTube, manage videos, track analytics, optimize metadata, and moderate comments from the same system.</p>
                </article>
                <article class="feature-card">
                    <div class="feature-icon">
                        <svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                            <rect x="5" y="11" width="14" height="10" rx="2"></rect>
                            <path d="M8 11V8a4 4 0 1 1 8 0v3"></path>
                        </svg>
                    </div>
                    <h3>Secure and Private</h3>
                    <p>Your videos, previews, and delivery workflows are processed with access control and production-minded backend handling.</p>
                </article>
                <article class="feature-card">
                    <div class="feature-icon">
                        <svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M4 12h16M12 4v16"></path>
                            <circle cx="12" cy="12" r="10"></circle>
                            <path d="M8 8 6 6M16 8l2-2M8 16l-2 2M16 16l2 2"></path>
                        </svg>
                    </div>
                    <h3>Cross-Platform Publishing</h3>
                    <p>Schedule and auto-publish clips, videos, and campaigns across YouTube, TikTok, Instagram, and X from one dashboard. Connect your accounts once — our Campaign Engine posts daily on your behalf.</p>
                </article>
            </div>
        </div>
    </section>

    <section id="tools" class="section section-dark">
        <div class="container">
            <div class="section-intro">
                <div class="section-kicker">Editing Engine</div>
                <h2>Professional Video Editing & Generation Engine</h2>
                <p class="tools-subcopy">Every tool is callable through natural language, so the same agent can move from editing to generation to final export without a context switch. Videos of any length — no 60-second cap.</p>
            </div>
            <div class="tools-grid">
                <article class="tool-category">
                    <h3><span class="tool-heading-icon"><svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h16M7 4v6M17 4v6M6 20l12-12M13 20l7-7"></path></svg></span>Core Editing</h3>
                    <ul class="tool-list">
                        <li>Trim, cut, merge, split</li>
                        <li>Deshake and stabilize</li>
                        <li>Reverse, loop, concatenate</li>
                        <li>Scene detection and analysis</li>
                        <li>Segment and chapter split</li>
                    </ul>
                </article>
                <article class="tool-category">
                    <h3><span class="tool-heading-icon"><svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="13.5" cy="6.5" r="2.5"></circle><circle cx="7.5" cy="17.5" r="2.5"></circle><circle cx="18" cy="16" r="2"></circle><path d="m10 8 1.5 6M15.5 8.5 16.8 14"></path></svg></span>Visual Effects</h3>
                    <ul class="tool-list">
                        <li>Color grading and LUT3D</li>
                        <li>Cinematic film grain and vignette</li>
                        <li>Chroma key and green screen</li>
                        <li>Motion blur, glow, bloom</li>
                        <li>Posterize, solarize, vibrance, HSV</li>
                    </ul>
                </article>
                <article class="tool-category">
                    <h3><span class="tool-heading-icon"><svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M11 5 6 9H3v6h3l5 4V5Z"></path><path d="M15.5 8.5a5 5 0 0 1 0 7M18 6a8.5 8.5 0 0 1 0 12"></path></svg></span>Audio Processing</h3>
                    <ul class="tool-list">
                        <li>Loudnorm, LUFS, and cleanup</li>
                        <li>RNN denoise and de-esser</li>
                        <li>Equalizer, compressor, limiter</li>
                        <li>Spectrum visualization</li>
                        <li>Pitch shift and time stretch</li>
                    </ul>
                </article>
                <article class="tool-category">
                    <h3><span class="tool-heading-icon"><svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 19h16"></path><path d="M7 16V9M12 16V5M17 16v-3"></path></svg></span>Analysis and Review</h3>
                    <ul class="tool-list">
                        <li>VMAF, SSIM, and PSNR quality checks</li>
                        <li>Loudness and silence detection</li>
                        <li>Scene change detection</li>
                        <li>Black frame and freeze detection</li>
                        <li>Bitrate and metadata extraction</li>
                    </ul>
                </article>
                <article class="tool-category">
                    <h3><span class="tool-heading-icon"><svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3h7v7"></path><path d="M10 14 21 3"></path><path d="M21 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5"></path></svg></span>Platform Export</h3>
                    <ul class="tool-list">
                        <li>YouTube, TikTok, Instagram</li>
                        <li>Format conversion across major codecs</li>
                        <li>H.264, H.265, VP9, AV1</li>
                        <li>GIF palette optimization</li>
                        <li>HDR to SDR tone mapping</li>
                    </ul>
                </article>
                <article class="tool-category">
                    <h3><span class="tool-heading-icon"><svg viewBox="0 0 24 24" fill="none" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h11"></path><path d="m12 3 4 4-4 4"></path><path d="M20 17H9"></path><path d="m12 13-4 4 4 4"></path></svg></span>Workflow Recipes</h3>
                    <ul class="tool-list">
                        <li>YouTube-ready export</li>
                        <li>Podcast audio cleanup</li>
                        <li>Cinematic grade</li>
                        <li>Talking-head cleanup</li>
                        <li>GIF creator</li>
                    </ul>
                </article>
            </div>
        </div>
    </section>

    <section id="about" class="section">
        <div class="container">
            <div class="section-intro">
                <div class="section-kicker">Built To Compound</div>
                <h2>One platform, multiple monetization layers</h2>
                <p>The same orchestration layer can edit creator videos, generate new motion scenes, package productized offers, and hand off polished deliverables through previews and wallet-based unlocks.</p>
            </div>
            <div class="about-grid">
                <article class="about-card">
                    <h3>Long-term recurring revenue</h3>
                    <p>The core business stays the creator and team subscription: a dependable workspace for AI editing, generation, export, and delivery that people can use every month.</p>
                    <h3>Short-term premium revenue</h3>
                    <p>When needed, the same system produces higher-ticket outputs like SaaS hero videos, website-driven animations, 3D mockups, and launch packs without introducing a second production stack.</p>
                    <h3>Agent-ready architecture</h3>
                    <p>Because professional editing, animated 3D/2D scenes, animated explainers, voice, stock footage, and delivery all sit behind one orchestration layer, improving the agents improves every offer at once.</p>
                </article>
                <div class="stat-stack">
                    <div class="stat-item">
                        <strong>$15</strong>
                        <span>Recurring creator plan that should stay the headline offer</span>
                    </div>
                    <div class="stat-item">
                        <strong>Any Length</strong>
                        <span>No 60-second cap — generate videos of any duration</span>
                    </div>
                    <div class="stat-item">
                        <strong>3D/2D Animation</strong>
                        <span>Professional animated scenes, mockups, title cards — generate videos of any length</span>
                    </div>
                </div>
            </div>
        </div>
    </section>

    <section class="section section-dark">
        <div class="container">
            <div class="cta-card">
                <h2>Start with the editor, then expand into premium offers</h2>
                <p>Use the recurring AI editing workflow as the stable foundation, then close higher-ticket launch packs, creator assets, or agency production from the same backend.</p>
                <a href="/signup" class="btn btn-primary btn-large">Start using VideoSync</a>
            </div>
        </div>
    </section>

    <footer class="footer">
        <div class="container">
            <div class="footer-content">
                <p>&copy; 2026 VideoSync. All rights reserved.</p>
                <div class="footer-links">
                    <a href="/privacy">Privacy Policy</a>
                    <span>|</span>
                    <a href="/terms">Terms of Service</a>
                    <span>|</span>
                    <a href="/help">Help Center</a>
                </div>
            </div>
        </div>
    </footer>

    <script>
        class DynamicBackgroundManager {
            constructor() {
                this.isUpdating = false;
                this.updateBackground();
                setInterval(() => this.updateBackground(), 5 * 60 * 1000);
            }

            async updateBackground() {
                if (this.isUpdating) return;
                this.isUpdating = true;

                try {
                    const response = await fetch('/api/background/image');
                    if (!response.ok) return;

                    const contentType = response.headers.get('content-type') || '';

                    if (contentType.includes('application/json')) {
                        const data = await response.json();
                        if (data.fallback && data.gradient) {
                            document.body.style.background = data.gradient;
                        }
                        return;
                    }

                    const blob = await response.blob();
                    const imageUrl = URL.createObjectURL(blob);
                    const overlay = document.createElement('div');
                    overlay.style.cssText = `
                        position: fixed;
                        inset: 0;
                        background-image: url(${imageUrl});
                        background-size: cover;
                        background-position: center;
                        background-attachment: fixed;
                        opacity: 0;
                        transition: opacity 1s ease-in-out;
                        z-index: -1;
                        pointer-events: none;
                    `;
                    document.body.appendChild(overlay);

                    setTimeout(() => {
                        overlay.style.opacity = '0.26';
                    }, 120);

                    setTimeout(() => {
                        const layers = Array.from(document.querySelectorAll('div[style*="background-image"]'));
                        layers.slice(0, -1).forEach(layer => layer.remove());
                    }, 1200);
                } catch (error) {
                    console.error('Failed to update landing background:', error);
                } finally {
                    this.isUpdating = false;
                }
            }
        }

        new DynamicBackgroundManager();
    </script>
<script>
function getStoredHomepageToken() {
    return localStorage.getItem('auth_token')
        || localStorage.getItem('authToken')
        || localStorage.getItem('admin_token')
        || '';
}
function swapHomepageCtasForAuthenticatedUser() {
    var t = getStoredHomepageToken();
    if (!t) return;
    var nb = document.getElementById('homepageAuthButtons');
    var hb = document.getElementById('homepageHeroButtons');
    if (nb) nb.innerHTML = '<a href="/dashboard" class="btn btn-primary">Dashboard</a><a href="/chat" class="btn btn-secondary">Open Chat</a>';
    if (hb) hb.innerHTML = '<a href="/chat" class="btn btn-primary btn-large">Open Chat</a><a href="/dashboard" class="btn btn-secondary btn-large">Dashboard</a>';
}
document.addEventListener('DOMContentLoaded', swapHomepageCtasForAuthenticatedUser);
swapHomepageCtasForAuthenticatedUser();
</script>
</body>
</html>
    "###
}

fn build_studio_landing_page_html() -> String {
    format!(r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Studio — VideoSync</title>
<style>
:root {{ --bg:#07111d; --panel:rgba(9,18,31,0.84); --line:rgba(148,163,184,0.16); --text:#e5eefb; --muted:#a8b8d3; --blue:#60a5fa; --green:#22c55e; }}
* {{ margin:0; padding:0; box-sizing:border-box; }}
body {{ font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; color:var(--text); background: radial-gradient(circle at top left, rgba(59,130,246,0.18), transparent 28%), linear-gradient(135deg,#0a1322 0%,#0d1728 55%,#07111d 100%); line-height:1.6; }}
.container {{ max-width:1200px; margin:0 auto; padding:0 20px; }}
.header {{ position:fixed; top:0; width:100%; z-index:1000; background:rgba(4,10,18,0.78); backdrop-filter:blur(18px); border-bottom:1px solid rgba(96,165,250,0.16); }}
.nav {{ min-height:78px; display:flex; justify-content:space-between; align-items:center; gap:1rem; }}
.logo {{ font-size:1.45rem; font-weight:800; text-decoration:none; letter-spacing:0.02em; color:var(--text); }}
.nav-links a, .auth-buttons a {{ text-decoration:none; color:#dbeafe; padding:0.55rem 0.95rem; border-radius:999px; transition:background .25s; }}
.nav-links a:hover, .auth-buttons a:hover {{ background:rgba(59,130,246,0.14); }}
.auth-buttons .cta {{ background:var(--blue); color:#000!important; font-weight:600; }}
.auth-buttons .cta:hover {{ background:#93bbfd!important; }}
.hero {{ padding:140px 0 60px; text-align:center; }}
.hero h1 {{ font-size:2.6rem; font-weight:800; line-height:1.2; background:linear-gradient(135deg,#e5eefb 0%,#60a5fa 100%); -webkit-background-clip:text; -webkit-text-fill-color:transparent; }}
.hero p {{ font-size:1.15rem; color:var(--muted); max-width:680px; margin:20px auto 32px; }}
.hero .sub {{ font-size:0.95rem; color:var(--muted); margin-top:8px; }}
.cta-row {{ display:flex; gap:12px; justify-content:center; flex-wrap:wrap; }}
.btn {{ display:inline-block; padding:12px 28px; border-radius:999px; font-weight:600; font-size:1rem; text-decoration:none; transition:all .2s; }}
.btn-primary {{ background:var(--blue); color:#000!important; }}
.btn-primary:hover {{ background:#93bbfd; transform:translateY(-1px); }}
.btn-outline {{ border:1px solid var(--line); color:var(--text)!important; }}
.btn-outline:hover {{ background:rgba(96,165,250,0.1); border-color:var(--blue); }}
.services {{ padding:40px 0 80px; }}
.services h2 {{ font-size:1.6rem; font-weight:700; margin-bottom:8px; }}
.services .intro {{ color:var(--muted); margin-bottom:32px; }}
.grid {{ display:grid; grid-template-columns:repeat(auto-fill,minmax(340px,1fr)); gap:16px; }}
.card {{ background:var(--panel); border:1px solid var(--line); border-radius:12px; padding:20px; transition:border .2s; }}
.card:hover {{ border-color:rgba(96,165,250,0.4); }}
.card h3 {{ font-size:1.05rem; font-weight:600; margin-bottom:4px; }}
.card .price {{ color:var(--green); font-weight:700; font-size:1.1rem; margin-bottom:8px; }}
.card p {{ color:var(--muted); font-size:0.9rem; margin-bottom:12px; }}
.card .tags {{ display:flex; gap:4px; flex-wrap:wrap; margin-bottom:12px; }}
.tag {{ background:rgba(96,165,250,0.12); color:var(--blue); padding:2px 8px; border-radius:999px; font-size:0.75rem; }}
.card .btn {{ display:inline-block; padding:8px 18px; border-radius:999px; font-size:0.85rem; font-weight:600; text-decoration:none; background:var(--blue); color:#000!important; }}
.footer {{ text-align:center; padding:40px 0; color:var(--muted); font-size:0.85rem; border-top:1px solid var(--line); }}
</style>
</head>
<body>
<div class="header">
<div class="container"><div class="nav">
<a href="/" class="logo">VideoSync Studio</a>
<div class="nav-links">
<a href="/services">All Services</a>
<a href="/subscribe">Subscribe</a>
<a href="/chat">Chat</a>
</div>
<div class="auth-buttons" id="homepageAuthButtons">
<a href="/login">Sign In</a>
<a href="/signup" class="cta">Get Started</a>
</div>
</div></div>
</div>
<div class="hero">
<div class="container">
<h1>Short-Term Services, AI-Produced in Hours</h1>
<p>Thumbnails, demo videos, explainers, mockups, voiceovers, and 3D scenes — all produced by our AI agent pipeline. No studio, no crew, no waiting weeks.</p>
<p class="sub">Produce more content for less. Our automated pipeline means 3 videos cost the same per-unit as 30. Traditional agencies charge 5-10x more for the same work.</p>
<div class="cta-row" style="margin-top:24px;">
<a href="/subscribe" class="btn btn-primary">Subscribe — $15/mo</a>
<a href="/services" class="btn btn-outline">Browse All Services</a>
</div>
</div>
</div>
<div class="services">
<div class="container">
<h2>Available Services</h2>
<p class="intro">Every service is available individually or through a subscription. AI-agent produced, delivered in hours, priced at a fraction of agency cost.</p>
<div class="grid">
<a href="/services/saas-launch-pack" class="card" style="text-decoration:none;color:inherit;">
<h3>SaaS/App Demo Rush</h3>
<div class="price">$399–$1,200</div>
<p>Polished product demo videos from your URL. AI agent produces in hours.</p>
<div class="tags"><span class="tag">Starter</span><span class="tag">Launch</span><span class="tag">Walkthrough</span></div>
<span class="btn">Order demo</span>
</a>
<a href="/services/clipper-enhancement-pack" class="card" style="text-decoration:none;color:inherit;">
<h3>Thumbnails & Motion Graphics</h3>
<div class="price">$250–$1,200</div>
<p>Thumbnails, title cards, lower thirds, device mockups — 10 variants in hours.</p>
<div class="tags"><span class="tag">YouTube</span><span class="tag">Campaign</span><span class="tag">Social</span></div>
<span class="btn">Order graphics</span>
</a>
<a href="/services/thumbnail-hero-pack" class="card" style="text-decoration:none;color:inherit;">
<h3>Thumbnail & Hero Visuals</h3>
<div class="price">$75–$300</div>
<p>Click-optimized thumbnails and hero visuals. Same-day delivery.</p>
<div class="tags"><span class="tag">Thumbnail</span><span class="tag">Hero</span><span class="tag">Ad</span></div>
<span class="btn">Order thumbnail</span>
</a>
<a href="/services/product-mockup-pack" class="card" style="text-decoration:none;color:inherit;">
<h3>Product Mockup Videos</h3>
<div class="price">$299–$900</div>
<p>Animated UI mockups and product walkthroughs from URLs or screenshots.</p>
<div class="tags"><span class="tag">UI</span><span class="tag">Mockup</span><span class="tag">Demo</span></div>
<span class="btn">Order mockup</span>
</a>
<a href="/services/education-explainer-pack" class="card" style="text-decoration:none;color:inherit;">
<h3>Education Explainers</h3>
<div class="price">$300–$1,500</div>
            <p>Animated visuals, diagrams, narration — full course curriculum produced automatically.</p>
<div class="tags"><span class="tag">Course</span><span class="tag">Explainer</span><span class="tag">Tutorial</span></div>
<span class="btn">Order explainer</span>
</a>
<a href="/services/blender-scene-pack" class="card" style="text-decoration:none;color:inherit;">
            <h3>3D/2D Animated Scenes</h3>
<div class="price">$500–$2,500</div>
<p>3D product scenes, animations, cinematic visuals — multiple angles in hours.</p>
<div class="tags"><span class="tag">3D</span><span class="tag">Animation</span><span class="tag">Product</span></div>
<span class="btn">Order scene</span>
</a>
<a href="/services/voice-audio-pack" class="card" style="text-decoration:none;color:inherit;">
<h3>Voice & Audio Production</h3>
<div class="price">$99–$750</div>
<p>Narration, voiceovers, summaries — produce 20 clips in the time it takes to record one.</p>
<div class="tags"><span class="tag">Voiceover</span><span class="tag">Audio</span><span class="tag">Narration</span></div>
<span class="btn">Order audio</span>
</a>
<a href="/services/mixed-agency-bundle" class="card" style="text-decoration:none;color:inherit;">
<h3>Agency 3-Pack</h3>
<div class="price">$1,500</div>
<p>3 client demo videos you can resell under your own brand. Scale to 30 clients.</p>
<div class="tags"><span class="tag">Agency</span><span class="tag">Resell</span><span class="tag">Bundle</span></div>
<span class="btn">Order 3-pack</span>
</a>
<a href="/services/creator-manager-fulfillment" class="card" style="text-decoration:none;color:inherit;">
<h3>Agency Production Backend</h3>
<div class="price">$999–$3,000/mo</div>
<p>Recurring fulfillment backend. Fulfill 30 client videos with the effort it took to produce 3.</p>
<div class="tags"><span class="tag">White-label</span><span class="tag">Recurring</span><span class="tag">API</span></div>
<span class="btn">Explore backend</span>
</a>
</div>
</div>
</div>
<div class="footer">
<div class="container">
<p>VideoSync Studio — AI-agent video production. Produce more content for less. No studio, no crew, no waiting.</p>
<p style="margin-top:8px;"><a href="/terms" style="color:var(--muted);">Terms</a> · <a href="/privacy" style="color:var(--muted);">Privacy</a></p>
</div>
</div>
<script>
(async function(){{
  const t=localStorage.getItem('authToken')||localStorage.getItem('admin_token')||localStorage.getItem('auth_token');
  const c=document.getElementById('homepageAuthButtons');
  if(t&&c)try{{
    const r=await fetch('/api/auth/verify',{{headers:{{'Authorization':'Bearer '+t}}}});
    if(r.ok){{
      const d=await r.json(),u=d.user||d;
      c.innerHTML='<span style="color:var(--muted);margin-right:8px">'+(u.email||u.username||'User')+'</span><a href="/dashboard" class="btn btn-secondary">Dashboard</a><a href='#' onclick="localStorage.clear();location.reload()" class="btn btn-secondary">Logout</a>';
    }}
  }}catch(e){{}}
}})();
</script>
</body>
</html>"###)
}

pub async fn login_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Login - VideoSync</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        html {
            scroll-behavior: smooth;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-size: cover;
            background-position: center;
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            color: #e8e8e8;
        }

        .auth-container {
            background: rgba(30, 30, 52, 0.8);
            backdrop-filter: blur(20px);
            border: 1px solid rgba(59, 130, 246, 0.3);
            padding: 3rem;
            border-radius: 20px;
            box-shadow: 0 20px 40px rgba(0,0,0,0.3);
            width: 100%;
            max-width: 400px;
            color: #e8e8e8;
            opacity: 0;
            transform: translateY(30px) scale(0.95);
            animation: authFormSlideIn 0.6s ease-out forwards;
        }

        @keyframes authFormSlideIn {
            to {
                opacity: 1;
                transform: translateY(0) scale(1);
            }
        }

        .auth-header {
            text-align: center;
            margin-bottom: 2rem;
        }

        .auth-header h1 {
            font-size: 2rem;
            color: #f8fafc;
            margin-bottom: 0.5rem;
        }

        .auth-header p {
            color: #cbd5e1;
        }

        .form-group {
            margin-bottom: 1.5rem;
        }

        .form-group label {
            display: block;
            margin-bottom: 0.5rem;
            color: #f8fafc;
            font-weight: 500;
        }

        .form-group input {
            width: 100%;
            padding: 0.75rem 1rem;
            border: 2px solid rgba(59, 130, 246, 0.3);
            border-radius: 10px;
            font-size: 1rem;
            background: rgba(15, 20, 25, 0.6);
            color: #e8e8e8;
            transition: all 0.3s ease;
        }

        .form-group input:focus {
            outline: none;
            border-color: #3b82f6;
            background: rgba(15, 20, 25, 0.8);
            transform: translateY(-2px);
            box-shadow: 0 4px 12px rgba(59, 130, 246, 0.3);
        }

        .form-group input::placeholder {
            color: #9ca3af;
        }

        .btn {
            width: 100%;
            padding: 0.75rem;
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            border: 1px solid rgba(59, 130, 246, 0.3);
            border-radius: 10px;
            font-size: 1rem;
            font-weight: 600;
            cursor: pointer;
            transition: all 0.3s ease;
        }

        .btn:hover {
            background: linear-gradient(135deg, #2563eb, #1e40af);
            transform: translateY(-2px);
            box-shadow: 0 4px 12px rgba(59, 130, 246, 0.4);
        }

        .auth-links {
            text-align: center;
            margin-top: 1.5rem;
        }

        .auth-links a {
            color: #3b82f6;
            text-decoration: none;
        }

        .auth-links a:hover {
            text-decoration: underline;
        }

        .error-message {
            background: #f8d7da;
            color: #721c24;
            padding: 0.75rem;
            border-radius: 8px;
            margin-bottom: 1rem;
            display: none;
        }

        .success-message {
            background: #d4edda;
            color: #155724;
            padding: 0.75rem;
            border-radius: 8px;
            margin-bottom: 1rem;
            display: none;
        }

        .back-link {
            position: absolute;
            top: 2rem;
            left: 2rem;
            color: #cbd5e1;
            text-decoration: none;
            font-weight: 500;
            transition: color 0.3s ease;
        }

        .back-link:hover {
            color: #3b82f6;
            text-decoration: underline;
        }

        .divider {
            text-align: center;
            margin: 1.5rem 0;
            position: relative;
        }

        .divider::before {
            content: '';
            position: absolute;
            left: 0;
            top: 50%;
            width: 100%;
            height: 1px;
            background: rgba(59, 130, 246, 0.3);
        }

        .divider span {
            background: rgba(30, 30, 52, 0.8);
            padding: 0 1rem;
            position: relative;
            color: #cbd5e1;
        }

        .btn-google {
            width: 100%;
            padding: 0.75rem;
            background: white;
            color: #444;
            border: 1px solid #ddd;
            border-radius: 10px;
            font-size: 1rem;
            font-weight: 600;
            cursor: pointer;
            transition: all 0.3s ease;
            display: flex;
            align-items: center;
            justify-content: center;
            gap: 0.75rem;
            margin-bottom: 1rem;
        }

        .btn-google:hover {
            background: #f8f9fa;
            border-color: #3b82f6;
            transform: translateY(-2px);
            box-shadow: 0 4px 12px rgba(59, 130, 246, 0.2);
        }

        .google-icon {
            width: 20px;
            height: 20px;
        }
    </style>
</head>
<body>
    <a href="/" class="back-link">← Back to Home</a>

    <div class="auth-container">
        <div class="auth-header">
            <h1>🎬 Welcome Back</h1>
            <p>Sign in to your account</p>
        </div>

        <div id="errorMessage" class="error-message"></div>
        <div id="successMessage" class="success-message"></div>

        <button onclick="signInWithGoogle()" class="btn-google">
            <svg class="google-icon" viewBox="0 0 48 48"><path fill="#EA4335" d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z"/><path fill="#4285F4" d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z"/><path fill="#FBBC05" d="M10.53 28.59c-.48-1.45-.76-2.99-.76-4.59s.27-3.14.76-4.59l-7.98-6.19C.92 16.46 0 20.12 0 24c0 3.88.92 7.54 2.56 10.78l7.97-6.19z"/><path fill="#34A853" d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z"/></svg>
            Sign in with Google
        </button>

        <div class="divider"><span>OR</span></div>

        <form id="loginForm">
            <div class="form-group">
                <label for="email">Email Address</label>
                <input type="email" id="email" name="email" required>
            </div>

            <div class="form-group">
                <label for="password">Password</label>
                <input type="password" id="password" name="password" required>
            </div>

            <button type="submit" class="btn">Sign In</button>
        </form>

        <div class="auth-links">
            <p>Don't have an account? <a href="/signup">Sign up here</a></p>
        </div>
    </div>

    <script>
        function getPostAuthRedirect(defaultPath) {
            const params = new URLSearchParams(window.location.search);
            return params.get('redirect_to') || defaultPath;
        }

        document.getElementById('loginForm').addEventListener('submit', async (e) => {
            e.preventDefault();
            
            const email = document.getElementById('email').value;
            const password = document.getElementById('password').value;
            
            try {
                const response = await fetch('/api/auth/login', {
                    method: 'POST',
                    headers: {
                        'Content-Type': 'application/json',
                    },
                    body: JSON.stringify({ email, password }),
                });
                
                const data = await response.json();
                
                if (data.success) {
                    localStorage.setItem('authToken', data.token);
                    localStorage.setItem('auth_token', data.token);
                    localStorage.setItem('user', JSON.stringify(data.user));
                    localStorage.setItem('auth_user', JSON.stringify(data.user));

                    document.getElementById('successMessage').textContent = 'Login successful! Redirecting...';
                    document.getElementById('successMessage').style.display = 'block';
                    document.getElementById('errorMessage').style.display = 'none';

                    setTimeout(() => {
                        // Clippers land on manual clipping, everyone else on dashboard
                        const defaultDest = data.user && data.user.is_clipper ? '/manual-clipping' : '/dashboard';
                        const dest = getPostAuthRedirect(defaultDest);
                        window.location.href = dest;
                    }, 1000);
                } else {
                    document.getElementById('errorMessage').textContent = data.message;
                    document.getElementById('errorMessage').style.display = 'block';
                    document.getElementById('successMessage').style.display = 'none';
                }
            } catch (error) {
                document.getElementById('errorMessage').textContent = 'Network error. Please try again.';
                document.getElementById('errorMessage').style.display = 'block';
                document.getElementById('successMessage').style.display = 'none';
            }
        });

        function signInWithGoogle() {
            const defaultDest = '/dashboard';
            const dest = getPostAuthRedirect(defaultDest);
            window.location.href = '/api/auth/google?redirect_to=' + encodeURIComponent(dest);
        }

        // Dynamic Background Management for Login Page
        class LoginDynamicBackgroundManager {
            constructor() {
                this.lastBackgroundUpdate = Date.now();
                this.updateInterval = 5 * 60 * 1000; // 5 minutes
                this.retryDelay = 30 * 1000; // 30 seconds on error
                this.isUpdating = false;
                
                this.init();
            }

            async init() {
                // Load initial background
                await this.updateBackground();
                
                // Set up periodic updates
                setInterval(() => {
                    this.checkAndUpdateBackground();
                }, 60 * 1000); // Check every minute
            }

            async checkAndUpdateBackground() {
                if (this.isUpdating) return;
                
                const timeSinceLastUpdate = Date.now() - this.lastBackgroundUpdate;
                if (timeSinceLastUpdate >= this.updateInterval) {
                    await this.updateBackground();
                }
            }

            async updateBackground() {
                if (this.isUpdating) return;
                
                this.isUpdating = true;
                
                try {
                    const response = await fetch('/api/background/image');
                    
                    if (response.ok) {
                        const contentType = response.headers.get('content-type');
                        
                        if (contentType && contentType.includes('application/json')) {
                            // Fallback gradient
                            const data = await response.json();
                            if (data.fallback && data.gradient) {
                                document.body.style.background = data.gradient;
                            }
                        } else {
                            // Image response
                            const blob = await response.blob();
                            const imageUrl = URL.createObjectURL(blob);
                            
                            // Create overlay for smooth transition
                            const overlay = document.createElement('div');
                            overlay.style.cssText = `
                                position: fixed;
                                top: 0;
                                left: 0;
                                width: 100%;
                                height: 100%;
                                background-image: url(${imageUrl});
                                background-size: cover;
                                background-position: center;
                                background-attachment: fixed;
                                opacity: 0;
                                transition: opacity 1s ease-in-out;
                                z-index: -1;
                                pointer-events: none;
                            `;
                            
                            document.body.appendChild(overlay);
                            
                            // Trigger fade in with subtle opacity for auth page
                            setTimeout(() => {
                                overlay.style.opacity = '0.15'; // Very subtle for auth pages
                            }, 100);
                            
                            // Clean up old overlays after transition
                            setTimeout(() => {
                                const oldOverlays = document.querySelectorAll('div[style*="background-image"]');
                                oldOverlays.forEach((old, index) => {
                                    if (index < oldOverlays.length - 1) {
                                        old.remove();
                                    }
                                });
                            }, 1100);
                        }
                        
                        this.lastBackgroundUpdate = Date.now();
                    }
                } catch (error) {
                    console.error('Error updating login background:', error);
                    setTimeout(() => {
                        this.lastBackgroundUpdate = Date.now() - this.updateInterval + this.retryDelay;
                    }, this.retryDelay);
                } finally {
                    this.isUpdating = false;
                }
            }
        }

        // Initialize dynamic background manager for login
        document.addEventListener('DOMContentLoaded', () => {
            new LoginDynamicBackgroundManager();
        });
    </script>
</body>
</html>
    "###;

    Html(html.to_string())
}

pub async fn signup_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Sign Up - VideoSync</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        html {
            scroll-behavior: smooth;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segui UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            color: #e8e8e8;
        }

        .auth-container {
            background: rgba(22, 33, 62, 0.85);
            backdrop-filter: blur(25px);
            -webkit-backdrop-filter: blur(25px);
            border: 1px solid rgba(59, 130, 246, 0.2);
            padding: 4rem 3rem; /* Added extra top/bottom padding as requested */
            margin: 2rem; /* Extra margin for mobile spacing */
            border-radius: 24px;
            box-shadow: 0 25px 50px rgba(0,0,0,0.4), 
                        0 0 0 1px rgba(255,255,255,0.05) inset;
            width: 100%;
            max-width: 420px;
            color: #e8e8e8;
            opacity: 0;
            transform: translateY(30px) scale(0.95);
            animation: authFormSlideIn 0.6s ease-out forwards;
        }

        @keyframes authFormSlideIn {
            to {
                opacity: 1;
                transform: translateY(0) scale(1);
            }
        }

        .auth-header {
            text-align: center;
            margin-bottom: 2rem;
        }

        .auth-header h1 {
            font-size: 2rem;
            color: #f8fafc;
            margin-bottom: 0.5rem;
        }

        .auth-header p {
            color: #cbd5e1;
        }

        .form-group {
            margin-bottom: 1.5rem;
        }

        .form-group label {
            display: block;
            margin-bottom: 0.5rem;
            color: #f8fafc;
            font-weight: 500;
        }

        .form-group input {
            width: 100%;
            padding: 0.75rem 1rem;
            border: 2px solid rgba(59, 130, 246, 0.3);
            border-radius: 10px;
            font-size: 1rem;
            background: rgba(15, 20, 25, 0.6);
            color: #e8e8e8;
            transition: all 0.3s ease;
        }

        .form-group input:focus {
            outline: none;
            border-color: #3b82f6;
            background: rgba(15, 20, 25, 0.9);
            transform: translateY(-1px);
            box-shadow: 0 8px 25px rgba(59, 130, 246, 0.3),
                        0 0 0 1px rgba(59, 130, 246, 0.1) inset;
        }

        .form-group input::placeholder {
            color: #9ca3af;
        }

        .btn {
            width: 100%;
            padding: 0.875rem;
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            border: 1px solid rgba(59, 130, 246, 0.3);
            border-radius: 12px;
            font-size: 1rem;
            font-weight: 600;
            cursor: pointer;
            transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
            backdrop-filter: blur(10px);
            -webkit-backdrop-filter: blur(10px);
        }

        .btn:hover {
            background: linear-gradient(135deg, #2563eb, #1e40af);
            transform: translateY(-2px);
            box-shadow: 0 8px 25px rgba(59, 130, 246, 0.4),
                        0 0 0 1px rgba(59, 130, 246, 0.2) inset;
        }

        .auth-links {
            text-align: center;
            margin-top: 1.5rem;
        }

        .auth-links a {
            color: #3b82f6;
            text-decoration: none;
            transition: all 0.3s ease;
        }

        .auth-links a:hover {
            color: #60a5fa;
            text-decoration: underline;
        }

        .error-message {
            background: rgba(248, 215, 218, 0.9);
            backdrop-filter: blur(10px);
            -webkit-backdrop-filter: blur(10px);
            color: #721c24;
            padding: 0.875rem;
            border-radius: 12px;
            margin-bottom: 1rem;
            border: 1px solid rgba(245, 198, 203, 0.3);
            display: none;
        }

        .success-message {
            background: rgba(212, 237, 218, 0.9);
            backdrop-filter: blur(10px);
            -webkit-backdrop-filter: blur(10px);
            color: #155724;
            padding: 0.875rem;
            border-radius: 12px;
            margin-bottom: 1rem;
            border: 1px solid rgba(195, 230, 203, 0.3);
            display: none;
        }

        .back-link {
            position: absolute;
            top: 2rem;
            left: 2rem;
            color: #cbd5e1;
            text-decoration: none;
            font-weight: 500;
            transition: color 0.3s ease;
            backdrop-filter: blur(10px);
            -webkit-backdrop-filter: blur(10px);
            padding: 0.5rem 1rem;
            border-radius: 8px;
            background: rgba(22, 33, 62, 0.3);
        }

        .back-link:hover {
            color: #3b82f6;
            background: rgba(22, 33, 62, 0.5);
            text-decoration: underline;
        }

        .password-requirements {
            font-size: 0.8rem;
            color: #94a3b8;
            margin-top: 0.25rem;
        }

        .divider {
            text-align: center;
            margin: 1.5rem 0;
            position: relative;
        }

        .divider::before {
            content: '';
            position: absolute;
            left: 0;
            top: 50%;
            width: 100%;
            height: 1px;
            background: rgba(59, 130, 246, 0.3);
        }

        .divider span {
            background: rgba(22, 33, 62, 0.85);
            padding: 0 1rem;
            position: relative;
            color: #cbd5e1;
        }

        .btn-google {
            width: 100%;
            padding: 0.875rem;
            background: white;
            color: #444;
            border: 1px solid #ddd;
            border-radius: 12px;
            font-size: 1rem;
            font-weight: 600;
            cursor: pointer;
            transition: all 0.3s ease;
            display: flex;
            align-items: center;
            justify-content: center;
            gap: 0.75rem;
            margin-bottom: 1rem;
        }

        .btn-google:hover {
            background: #f8f9fa;
            border-color: #3b82f6;
            transform: translateY(-2px);
            box-shadow: 0 8px 25px rgba(59, 130, 246, 0.2);
        }

        .google-icon {
            width: 20px;
            height: 20px;
        }
    </style>
</head>
<body>
    <a href="/" class="back-link">← Back to Home</a>

    <div class="auth-container">
        <div class="auth-header">
            <h1>🎬 Get Started</h1>
            <p>Create your account</p>
        </div>

        <div id="errorMessage" class="error-message"></div>
        <div id="successMessage" class="success-message"></div>

        <button onclick="signUpWithGoogle()" class="btn-google">
            <svg class="google-icon" viewBox="0 0 48 48"><path fill="#EA4335" d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z"/><path fill="#4285F4" d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z"/><path fill="#FBBC05" d="M10.53 28.59c-.48-1.45-.76-2.99-.76-4.59s.27-3.14.76-4.59l-7.98-6.19C.92 16.46 0 20.12 0 24c0 3.88.92 7.54 2.56 10.78l7.97-6.19z"/><path fill="#34A853" d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z"/></svg>
            Sign up with Google
        </button>

        <div class="divider"><span>OR</span></div>

        <form id="signupForm">
            <div class="form-group">
                <label for="email">Email Address</label>
                <input type="email" id="email" name="email" required>
            </div>

            <div class="form-group">
                <label for="username">Username</label>
                <input type="text" id="username" name="username" required>
            </div>

            <div class="form-group">
                <label for="password">Password</label>
                <input type="password" id="password" name="password" required>
                <div class="password-requirements">
                    Must be at least 6 characters long
                </div>
            </div>

            <div class="form-group">
                <label for="confirmPassword">Confirm Password</label>
                <input type="password" id="confirmPassword" name="confirmPassword" required>
            </div>

            <button type="submit" class="btn">Create Account</button>
        </form>

        <div class="auth-links">
            <p>Already have an account? <a href="/login">Sign in here</a></p>
        </div>
    </div>

    <script>
        function getPostAuthRedirect(defaultPath) {
            const params = new URLSearchParams(window.location.search);
            return params.get('redirect_to') || defaultPath;
        }

        document.getElementById('signupForm').addEventListener('submit', async (e) => {
            e.preventDefault();
            
            const email = document.getElementById('email').value;
            const username = document.getElementById('username').value;
            const password = document.getElementById('password').value;
            const confirmPassword = document.getElementById('confirmPassword').value;
            
            if (password !== confirmPassword) {
                document.getElementById('errorMessage').textContent = 'Passwords do not match.';
                document.getElementById('errorMessage').style.display = 'block';
                document.getElementById('successMessage').style.display = 'none';
                return;
            }
            
            if (password.length < 6) {
                document.getElementById('errorMessage').textContent = 'Password must be at least 6 characters long.';
                document.getElementById('errorMessage').style.display = 'block';
                document.getElementById('successMessage').style.display = 'none';
                return;
            }
            
            try {
                const response = await fetch('/api/auth/register', {
                    method: 'POST',
                    headers: {
                        'Content-Type': 'application/json',
                    },
                    body: JSON.stringify({ email, username, password, confirm_password: confirmPassword }),
                });
                const raw = await response.text();
                let data = {};
                try {
                    data = raw ? JSON.parse(raw) : {};
                } catch (_) {
                    data = { success: false, message: raw || 'Registration failed.' };
                }
                
                if (response.ok && data.success) {
                    localStorage.setItem('authToken', data.token);
                    localStorage.setItem('auth_token', data.token);
                    localStorage.setItem('user', JSON.stringify(data.user));
                    localStorage.setItem('auth_user', JSON.stringify(data.user));
                    
                    document.getElementById('successMessage').textContent = 'Account created successfully! Redirecting...';
                    document.getElementById('successMessage').style.display = 'block';
                    document.getElementById('errorMessage').style.display = 'none';

                    setTimeout(() => {
                        const defaultDest = data.user && data.user.is_clipper ? '/manual-clipping' : '/dashboard';
                        const dest = getPostAuthRedirect(defaultDest);
                        window.location.href = dest;
                    }, 1000);
                } else {
                    document.getElementById('errorMessage').textContent = data.message || 'Registration failed.';
                    document.getElementById('errorMessage').style.display = 'block';
                    document.getElementById('successMessage').style.display = 'none';
                }
            } catch (error) {
                document.getElementById('errorMessage').textContent = 'Sign-up could not be completed right now. Please try again.';
                document.getElementById('errorMessage').style.display = 'block';
                document.getElementById('successMessage').style.display = 'none';
            }
        });

        function signUpWithGoogle() {
            const defaultDest = '/dashboard';
            const dest = getPostAuthRedirect(defaultDest);
            window.location.href = '/api/auth/google?redirect_to=' + encodeURIComponent(dest);
        }

        // Dynamic Background Manager Class for Signup
        class SignupDynamicBackgroundManager {
            constructor() {
                this.updateInterval = 5 * 60 * 1000; // 5 minutes
                this.retryDelay = 30 * 1000; // 30 seconds retry on error
                this.lastBackgroundUpdate = 0;
                this.isUpdating = false;
                
                // Initial background update
                setTimeout(() => this.updateBackground(), 1000);
                
                // Set up periodic updates
                setInterval(() => {
                    if (Date.now() - this.lastBackgroundUpdate >= this.updateInterval) {
                        this.updateBackground();
                    }
                }, 30000); // Check every 30 seconds
            }
            
            async updateBackground() {
                if (this.isUpdating) return;
                
                this.isUpdating = true;
                
                try {
                    const response = await fetch('/api/background/image');
                    
                    if (response.ok) {
                        const contentType = response.headers.get('content-type');
                        
                        if (contentType && contentType.includes('application/json')) {
                            // Fallback gradient
                            const data = await response.json();
                            if (data.fallback && data.gradient) {
                                document.body.style.background = data.gradient;
                            }
                        } else {
                            // Image response
                            const blob = await response.blob();
                            const imageUrl = URL.createObjectURL(blob);
                            
                            // Create overlay for smooth transition
                            const overlay = document.createElement('div');
                            overlay.style.cssText = `
                                position: fixed;
                                top: 0;
                                left: 0;
                                width: 100%;
                                height: 100%;
                                background-image: url(${imageUrl});
                                background-size: cover;
                                background-position: center;
                                background-attachment: fixed;
                                opacity: 0;
                                transition: opacity 1s ease-in-out;
                                z-index: -1;
                                pointer-events: none;
                            `;
                            
                            document.body.appendChild(overlay);
                            
                            // Trigger fade in with subtle opacity for auth page
                            setTimeout(() => {
                                overlay.style.opacity = '0.15'; // Very subtle for auth pages
                            }, 100);
                            
                            // Clean up old overlays after transition
                            setTimeout(() => {
                                const oldOverlays = document.querySelectorAll('div[style*="background-image"]');
                                oldOverlays.forEach((old, index) => {
                                    if (index < oldOverlays.length - 1) {
                                        old.remove();
                                    }
                                });
                            }, 1100);
                        }
                        
                        this.lastBackgroundUpdate = Date.now();
                    }
                } catch (error) {
                    console.error('Error updating signup background:', error);
                    setTimeout(() => {
                        this.lastBackgroundUpdate = Date.now() - this.updateInterval + this.retryDelay;
                    }, this.retryDelay);
                } finally {
                    this.isUpdating = false;
                }
            }
        }

        // Initialize dynamic background manager for signup
        document.addEventListener('DOMContentLoaded', () => {
            new SignupDynamicBackgroundManager();
        });
    </script>
</body>
</html>
    "###;

    Html(html.to_string())
}

pub async fn dashboard_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Dashboard - VideoSync</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        html {
            scroll-behavior: smooth;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            line-height: 1.6;
            color: #e8e8e8;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-size: cover;
            background-position: center;
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
            min-height: 100vh;
        }

        .header {
            background: rgba(26, 26, 46, 0.9);
            backdrop-filter: blur(10px);
            border-bottom: 1px solid rgba(59, 130, 246, 0.3);
            padding: 1rem 0;
        }

        .nav {
            max-width: 1200px;
            margin: 0 auto;
            padding: 0 20px;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .logo {
            font-size: 1.5rem;
            font-weight: bold;
            color: white;
            text-decoration: none;
        }

        .user-menu {
            display: flex;
            align-items: center;
            gap: 1rem;
        }

        .btn {
            padding: 0.5rem 1rem;
            border: none;
            border-radius: 8px;
            text-decoration: none;
            font-weight: 500;
            cursor: pointer;
            transition: all 0.3s;
        }

        .btn-primary {
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            border: 1px solid rgba(59, 130, 246, 0.3);
        }

        .btn-primary:hover {
            background: linear-gradient(135deg, #2563eb, #1e40af);
            transform: translateY(-2px);
            box-shadow: 0 4px 20px rgba(59, 130, 246, 0.4);
        }

        .btn-secondary {
            background: rgba(30, 30, 52, 0.8);
            color: #e8e8e8;
            border: 2px solid rgba(59, 130, 246, 0.3);
        }

        .btn-secondary:hover {
            background: rgba(59, 130, 246, 0.2);
            border-color: rgba(59, 130, 246, 0.6);
        }

        .container {
            max-width: 1200px;
            margin: 0 auto;
            padding: 2rem 20px;
        }

        .dashboard-header {
            margin-bottom: 3rem;
            text-align: center;
        }

        .dashboard-header h1 {
            font-size: 3rem;
            color: white;
            font-weight: 800;
            margin-bottom: 0.5rem;
            letter-spacing: -0.5px;
            text-shadow: 0 2px 20px rgba(102, 126, 234, 0.5);
        }

        .dashboard-header p {
            color: rgba(255, 255, 255, 0.8);
            font-size: 1.2rem;
            font-weight: 300;
        }

        .quick-actions {
            background: rgba(255, 255, 255, 0.05);
            backdrop-filter: blur(20px);
            border-radius: 24px;
            padding: 2.5rem;
            box-shadow: 0 20px 60px rgba(0,0,0,0.3);
            margin-bottom: 3rem;
            border: 1px solid rgba(59, 130, 246, 0.2);
        }

        .quick-actions h2 {
            margin-bottom: 2rem;
            color: white;
            font-size: 1.8rem;
            font-weight: 700;
        }

        .action-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 1.5rem;
        }

        .action-card {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 2.5rem;
            border-radius: 20px;
            text-decoration: none;
            transition: all 0.4s cubic-bezier(0.4, 0, 0.2, 1);
            opacity: 0;
            transform: translateY(30px);
            animation: dashboardCardSlideIn 0.7s ease-out forwards;
            position: relative;
            overflow: hidden;
            box-shadow: 0 10px 40px rgba(102, 126, 234, 0.3);
        }

        .action-card::before {
            content: '';
            position: absolute;
            top: 0;
            left: 0;
            right: 0;
            bottom: 0;
            background: linear-gradient(135deg, rgba(255,255,255,0.1) 0%, rgba(255,255,255,0) 100%);
            opacity: 0;
            transition: opacity 0.3s;
        }

        .action-card:hover::before {
            opacity: 1;
        }

        .action-card:nth-child(1) { animation-delay: 0.1s; background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); }
        .action-card:nth-child(2) { animation-delay: 0.2s; background: linear-gradient(135deg, #f093fb 0%, #f5576c 100%); }
        .action-card:nth-child(3) { animation-delay: 0.3s; background: linear-gradient(135deg, #4facfe 0%, #00f2fe 100%); }
        .action-card:nth-child(4) { animation-delay: 0.4s; background: linear-gradient(135deg, #43e97b 0%, #38f9d7 100%); }

        .action-card:hover {
            transform: translateY(-10px) scale(1.03);
            color: white;
            box-shadow: 0 20px 60px rgba(102, 126, 234, 0.5);
        }

        @keyframes dashboardCardSlideIn {
            to {
                opacity: 1;
                transform: translateY(0);
            }
        }

        .action-card h3 {
            margin-bottom: 0.5rem;
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }

        .action-icon {
            display: inline-flex;
            align-items: center;
            justify-content: center;
            width: 1.3rem;
            height: 1.3rem;
            flex-shrink: 0;
        }

        .action-icon svg {
            width: 100%;
            height: 100%;
            fill: none;
            stroke: currentColor;
            stroke-width: 1.9;
            stroke-linecap: round;
            stroke-linejoin: round;
        }

        .recent-chats {
            background: rgba(255, 255, 255, 0.05);
            backdrop-filter: blur(20px);
            border-radius: 24px;
            padding: 2.5rem;
            box-shadow: 0 20px 60px rgba(0,0,0,0.3);
            border: 1px solid rgba(59, 130, 246, 0.2);
        }

        .recent-chats h2 {
            margin-bottom: 2rem;
            color: white;
            font-size: 1.8rem;
            font-weight: 700;
        }

        .tabs {
            display: flex;
            gap: 1rem;
            margin-bottom: 2rem;
            border-bottom: 2px solid rgba(59, 130, 246, 0.2);
        }

        .tab-button {
            padding: 1rem 2rem;
            background: transparent;
            border: none;
            color: rgba(255, 255, 255, 0.6);
            cursor: pointer;
            font-size: 1rem;
            font-weight: 600;
            transition: all 0.3s;
            border-bottom: 3px solid transparent;
            margin-bottom: -2px;
        }

        .tab-button:hover {
            color: rgba(255, 255, 255, 0.9);
        }

        .tab-button.active {
            color: #3b82f6;
            border-bottom-color: #3b82f6;
        }

        .tab-content {
            display: none;
        }

        .tab-content.active {
            display: block;
        }

        .chat-list {
            list-style: none;
        }

        .chat-item {
            padding: 1.5rem;
            background: rgba(255, 255, 255, 0.03);
            backdrop-filter: blur(10px);
            border: 1px solid rgba(59, 130, 246, 0.15);
            display: flex;
            justify-content: space-between;
            align-items: center;
            transition: all 0.3s;
            border-radius: 12px;
            margin-bottom: 0.75rem;
            cursor: pointer;
        }

        .chat-item:hover {
            background: rgba(59, 130, 246, 0.15);
            backdrop-filter: blur(15px);
            transform: translateX(5px);
            border-left: 3px solid #3b82f6;
            border-color: rgba(59, 130, 246, 0.4);
            box-shadow: 0 4px 15px rgba(59, 130, 246, 0.2);
        }

        .chat-item:last-child {
            border-bottom: none;
        }

        .chat-info {
            flex: 1;
        }

        .chat-title {
            font-weight: 600;
            color: #e8e8e8;
            margin-bottom: 0.25rem;
        }

        .chat-time {
            font-size: 0.875rem;
            color: rgba(255, 255, 255, 0.6);
        }

        .chat-date {
            color: rgba(255, 255, 255, 0.6);
            font-size: 0.9rem;
        }

        .empty-state {
            text-align: center;
            color: rgba(255, 255, 255, 0.6);
            padding: 3rem;
        }

        .empty-state h3 {
            margin-bottom: 1rem;
            color: white;
        }
    </style>
</head>
<body>
    <header class="header">
        <div class="nav">
            <a href="/dashboard" class="logo">🎬 Agentic Video Editor</a>
            <div class="user-menu">
                <span id="userWelcome">Welcome back!</span>
                <button onclick="logout()" class="btn btn-secondary">Logout</button>
            </div>
        </div>
    </header>

    <div class="container">
        <div class="dashboard-header">
            <h1>Your Dashboard</h1>
            <p>Manage your video editing projects and start new conversations with our AI assistant.</p>
        </div>

        <div style="display:flex;flex-wrap:wrap;gap:10px;margin:0 0 24px;">
            <a href="/services" style="text-decoration:none;padding:10px 14px;border-radius:999px;background:rgba(59,130,246,0.18);border:1px solid rgba(96,165,250,0.28);color:#dbeafe;font-weight:600;">All Services</a>
            <a href="/services/saas-launch-pack" style="text-decoration:none;padding:10px 14px;border-radius:999px;background:rgba(15,23,42,0.72);border:1px solid rgba(148,163,184,0.18);color:#dbeafe;">SaaS Launch Pack</a>
            <a href="/services/clipper-enhancement-pack" style="text-decoration:none;padding:10px 14px;border-radius:999px;background:rgba(15,23,42,0.72);border:1px solid rgba(148,163,184,0.18);color:#dbeafe;">Thumbnail & Motion Graphics</a>
            <a href="/services/creator-manager-fulfillment" style="text-decoration:none;padding:10px 14px;border-radius:999px;background:rgba(15,23,42,0.72);border:1px solid rgba(148,163,184,0.18);color:#dbeafe;">Agency Production Backend</a>
            <a href="/services/x402-asset-api" style="text-decoration:none;padding:10px 14px;border-radius:999px;background:rgba(15,23,42,0.72);border:1px solid rgba(148,163,184,0.18);color:#dbeafe;">Programmable Payments</a>
        </div>

        <div class="quick-actions">
            <h2>Quick Actions</h2>
            <div class="action-grid">
                <a href="/chat" class="action-card">
                    <h3><span class="action-icon"><svg viewBox="0 0 24 24"><path d="M7 10h10"></path><path d="M7 14h6"></path><path d="M5 19V6a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H9l-4 3Z"></path></svg></span>Start New Chat</h3>
                    <p>Begin a new video editing session with our AI assistant</p>
                </a>
                <a href="/youtube/manage" class="action-card">
                    <h3><span class="action-icon"><svg viewBox="0 0 24 24"><path d="M3 7.5a2.5 2.5 0 0 1 2.5-2.5h9A2.5 2.5 0 0 1 17 7.5v9a2.5 2.5 0 0 1-2.5 2.5h-9A2.5 2.5 0 0 1 3 16.5Z"></path><path d="m10 9 4 3-4 3Z"></path><path d="M17 10.5 21 8v8l-4-2.5"></path></svg></span>Connect YouTube Channels</h3>
                    <p>Connect and manage your YouTube channels for seamless publishing</p>
                </a>
                <a href="/analytics" class="action-card">
                    <h3><span class="action-icon"><svg viewBox="0 0 24 24"><path d="M4 19h16"></path><path d="M7 16V9"></path><path d="M12 16V5"></path><path d="M17 16v-4"></path></svg></span>Analytics Dashboard</h3>
                    <p>View YouTube channel performance and video analytics</p>
                </a>
                <!-- YouTube Clipping Card (only for admins/whitelisted users) -->
                <a href="/clipping/manage" class="action-card" id="clipping-action-card" style="display: none;">
                    <h3><span class="action-icon"><svg viewBox="0 0 24 24"><circle cx="6" cy="6" r="2"></circle><circle cx="6" cy="18" r="2"></circle><path d="M8 7.5 19 4"></path><path d="M8 16.5 19 20"></path><path d="M14 12h7"></path></svg></span>YouTube Clipping</h3>
                    <p>Auto-generate viral clips from popular channels and post to your channel</p>
                </a>
                <a href="/video-tools" class="action-card">
                    <h3><span class="action-icon"><svg viewBox="0 0 24 24"><path d="m14.7 6.3 3 3"></path><path d="M4 20l4.5-1 9.2-9.2a2.1 2.1 0 1 0-3-3L5.5 16 4 20Z"></path><path d="M13 8 16 11"></path></svg></span>Video Tools</h3>
                    <p>Stabilize, convert formats, visualize audio, and run workflow recipes directly</p>
                </a>
                <a href="/gig-templates" class="action-card" id="gig-templates-action-card" style="display: none;">
                    <h3><span class="action-icon"><svg viewBox="0 0 24 24"><path d="M4 8h16v10a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2Z"></path><path d="M9 8V6a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2"></path><path d="M4 12h16"></path></svg></span>Gig Templates</h3>
                    <p>Fiverr & PPH gig info with pricing tiers, copy-paste descriptions, and AI sample video generation</p>
                </a>
                <a href="/manual-clipping" class="action-card">
                    <h3><span class="action-icon"><svg viewBox="0 0 24 24"><circle cx="6" cy="6" r="2"></circle><circle cx="6" cy="18" r="2"></circle><path d="M8 7.5 19 4"></path><path d="M8 16.5 19 20"></path><path d="M14 12h7"></path></svg></span>Manual Clipping</h3>
                    <p>Paste any YouTube or Twitch URL to extract viral clips with download links — no destination channel needed</p>
                </a>
                <a href="/api/campaigns" class="action-card">
                    <h3><span class="action-icon"><svg viewBox="0 0 24 24"><rect x="3" y="3" width="6" height="6" rx="1"></rect><rect x="15" y="3" width="6" height="6" rx="1"></rect><rect x="3" y="15" width="6" height="6" rx="1"></rect><rect x="15" y="15" width="6" height="6" rx="1"></rect></svg></span>Campaigns</h3>
                    <p>Set up automated daily content generation and cross-platform posting campaigns</p>
                </a>
                <a href="/help" class="action-card">
                    <h3><span class="action-icon"><svg viewBox="0 0 24 24"><path d="M5 4h11a3 3 0 0 1 3 3v13H8a3 3 0 0 0-3 3Z"></path><path d="M8 4v16a3 3 0 0 0-3 3V7a3 3 0 0 1 3-3Z"></path><path d="M11 9h4"></path><path d="M11 13h4"></path></svg></span>Help & Guide</h3>
                    <p>Learn how to use the AI video editor and YouTube features</p>
                </a>
            </div>
        </div>

        <div class="recent-chats">
            <div class="tabs">
                <button class="tab-button active" onclick="switchTab('recent')">Recent Chats (Last 10)</button>
                <button class="tab-button" onclick="switchTab('all')">All Chats</button>
            </div>

            <div id="recentTab" class="tab-content active">
                <div id="chatList">
                    <div class="empty-state">
                        <h3>No chats yet</h3>
                        <p>Start your first conversation with our AI assistant to see your chat history here.</p>
                        <a href="/chat" class="btn btn-primary" style="margin-top: 1rem; display: inline-block;">Start First Chat</a>
                    </div>
                </div>
            </div>

            <div id="allTab" class="tab-content">
                <div id="allChatsList">
                    <div class="loading">Loading all chats...</div>
                </div>
                <div id="pagination" style="display: none; text-align: center; margin-top: 20px;">
                    <button onclick="loadPage(currentPage - 1)" id="prevBtn" class="btn btn-secondary" style="margin: 0 5px;">Previous</button>
                    <span id="pageInfo" style="margin: 0 15px; color: #e8e8e8;"></span>
                    <button onclick="loadPage(currentPage + 1)" id="nextBtn" class="btn btn-secondary" style="margin: 0 5px;">Next</button>
                </div>
            </div>
        </div>
    </div>

    <script>
        // Check authentication
        const authToken = localStorage.getItem('authToken') || localStorage.getItem('admin_token') || localStorage.getItem('auth_token');
        if (!authToken) {
            window.location.href = '/login';
        }

        // Clippers should use manual clipping dashboard, not this page
        function parseJwt(token) {
            try {
                return JSON.parse(atob(token.split('.')[1]));
            } catch (_) {
                return {};
            }
        }

        const storedUser = JSON.parse(localStorage.getItem('user') || localStorage.getItem('auth_user') || '{}');
        const tokenClaims = parseJwt(authToken);
        const user = { ...storedUser, ...tokenClaims };
        if (user.is_clipper) {
            window.location.href = '/manual-clipping';
        }

        // Set user welcome message
        if (user.username) {
            document.getElementById('userWelcome').textContent = `Welcome back, ${user.username}!`;
        }

        // Show/hide YouTube clipping card based on permissions
        // Check access via API to include whitelisted users
        const clippingCard = document.getElementById('clipping-action-card');
        const gigTemplatesCard = document.getElementById('gig-templates-action-card');
        if (clippingCard || gigTemplatesCard) {
            const authToken = localStorage.getItem('authToken') || localStorage.getItem('admin_token') || localStorage.getItem('auth_token');
            if (authToken) {
                fetch('/api/clipping/access-check', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                })
                .then(response => {
                    if (response.ok) {
                        if (clippingCard) clippingCard.style.display = 'block';
                        if (gigTemplatesCard) gigTemplatesCard.style.display = 'block';
                    }
                })
                .catch(err => console.debug('Clipping access check failed:', err));
            }
        }

        function logout() {
            localStorage.removeItem('authToken');
            localStorage.removeItem('auth_token');
            localStorage.removeItem('admin_token');
            localStorage.removeItem('user');
            localStorage.removeItem('auth_user');
            window.location.href = '/';
        }

        function uploadVideo() {
            window.location.href = '/chat?action=upload';
        }

        function viewProjects() {
            window.location.href = '/dashboard';
        }

        function viewHelp() {
            window.location.href = '/help';
        }

        function escapeHtml(value) {
            return String(value || '')
                .replace(/&/g, '&amp;')
                .replace(/</g, '&lt;')
                .replace(/>/g, '&gt;')
                .replace(/"/g, '&quot;')
                .replace(/'/g, '&#39;');
        }

        function renderChatItem(chat, includeMessageCount = false) {
            const meta = includeMessageCount
                ? `${new Date(chat.created_at).toLocaleString()} • ${chat.message_count} messages`
                : new Date(chat.created_at).toLocaleString();
            const serializedTitle = JSON.stringify(chat.title || '');
            return `
                <div class="chat-item" onclick="openChat('${chat.session_id}')">
                    <div class="chat-info">
                        <div class="chat-title">${escapeHtml(chat.title)}</div>
                        <div class="chat-time">${meta}</div>
                    </div>
                    <button class="btn btn-secondary" style="padding:6px 10px;font-size:12px;" onclick='renameChat(event, "${chat.session_id}", ${serializedTitle})'>Rename</button>
                </div>
            `;
        }

        async function renameChat(event, sessionId, currentTitle) {
            event.stopPropagation();
            const nextTitle = prompt('Rename this chat:', currentTitle || '');
            if (nextTitle === null) return;

            const trimmed = nextTitle.trim();
            if (!trimmed) {
                alert('Title cannot be empty.');
                return;
            }

            const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
            const response = await fetch(`/api/chat/sessions/${sessionId}/title`, {
                method: 'POST',
                headers: {
                    'Authorization': `Bearer ${authToken}`,
                    'Content-Type': 'application/json'
                },
                body: JSON.stringify({ title: trimmed })
            });

            if (!response.ok) {
                alert('Failed to rename chat.');
                return;
            }

            loadRecentChats();
            if (document.getElementById('allTab').classList.contains('active')) {
                loadAllChats(currentPage);
            }
        }

        // Load recent chats
        async function loadRecentChats() {
            try {
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
                const response = await fetch('/api/chat/recent', {
                    headers: {
                        'Authorization': `Bearer ${authToken}`
                    }
                });

                if (response.ok) {
                    const data = await response.json();
                    const chatList = document.getElementById('chatList');
                    
                    if (data.success && data.chats && data.chats.length > 0) {
                        chatList.innerHTML = data.chats.map(chat => renderChatItem(chat)).join('');
                    } else {
                        // Keep the empty state if no chats
                        chatList.innerHTML = `
                            <div class="empty-state">
                                <h3>No chats yet</h3>
                                <p>Start your first conversation with our AI assistant to see your chat history here.</p>
                                <a href="/chat" class="btn btn-primary" style="margin-top: 1rem; display: inline-block;">Start First Chat</a>
                            </div>
                        `;
                    }
                } else {
                    console.error('Failed to load recent chats');
                }
            } catch (error) {
                console.error('Error loading recent chats:', error);
            }
        }

        function openChat(sessionId) {
            window.location.href = `/chat/${sessionId}`;
        }

        // Tab switching
        let currentPage = 1;
        let totalPages = 1;

        function switchTab(tabName) {
            // Update tab buttons
            document.querySelectorAll('.tab-button').forEach(btn => btn.classList.remove('active'));
            event.target.classList.add('active');

            // Update tab content
            document.querySelectorAll('.tab-content').forEach(content => content.classList.remove('active'));

            if (tabName === 'recent') {
                document.getElementById('recentTab').classList.add('active');
            } else if (tabName === 'all') {
                document.getElementById('allTab').classList.add('active');
                loadAllChats(1);
            }
        }

        // Load all chats with pagination
        async function loadAllChats(page = 1) {
            try {
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
                const response = await fetch(`/api/chat/all?page=${page}&limit=20`, {
                    headers: {
                        'Authorization': `Bearer ${authToken}`
                    }
                });

                if (response.ok) {
                    const data = await response.json();
                    const allChatsList = document.getElementById('allChatsList');
                    const paginationDiv = document.getElementById('pagination');

                    if (data.success && data.chats && data.chats.length > 0) {
                        allChatsList.innerHTML = data.chats.map(chat => renderChatItem(chat, true)).join('');

                        // Update pagination
                        currentPage = data.pagination.page;
                        totalPages = data.pagination.total_pages;

                        document.getElementById('pageInfo').textContent = `Page ${currentPage} of ${totalPages} (${data.pagination.total} total chats)`;
                        document.getElementById('prevBtn').disabled = currentPage <= 1;
                        document.getElementById('nextBtn').disabled = currentPage >= totalPages;
                        paginationDiv.style.display = totalPages > 1 ? 'block' : 'none';
                    } else {
                        allChatsList.innerHTML = `
                            <div class="empty-state">
                                <h3>No chats yet</h3>
                                <p>Start your first conversation with our AI assistant.</p>
                                <a href="/chat" class="btn btn-primary" style="margin-top: 1rem; display: inline-block;">Start First Chat</a>
                            </div>
                        `;
                        paginationDiv.style.display = 'none';
                    }
                } else {
                    console.error('Failed to load all chats');
                }
            } catch (error) {
                console.error('Error loading all chats:', error);
            }
        }

        function loadPage(page) {
            if (page >= 1 && page <= totalPages) {
                loadAllChats(page);
            }
        }

        loadRecentChats();
    </script>
<script>
class DynamicBackgroundManager {
    constructor() { this.lastUpdate = Date.now(); this.interval = 5*60*1000; this.init(); }
    async init() { await this.updateBg(); setInterval(() => this.updateBg(), this.interval); }
    async updateBg() {
        try {
            const r = await fetch('/api/background/image');
            if (!r.ok) return;
            const ct = r.headers.get('content-type') || '';
            if (ct.includes('application/json')) {
                const d = await r.json();
                if (d.fallback && d.gradient) document.body.style.background = d.gradient;
                return;
            }
            const blob = await r.blob();
            const url = URL.createObjectURL(blob);
            const o = document.createElement('div');
            o.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;background-image:url('+url+');background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none';
            document.body.appendChild(o);
            setTimeout(() => o.style.opacity = '0.3', 100);
            setTimeout(() => {
                const old = document.querySelectorAll('div[style*="background-image"]');
                old.forEach((e,i) => { if (i < old.length - 1) e.remove(); });
            }, 1100);
        } catch(e) { console.error(e); }
    }
}
new DynamicBackgroundManager();
</script>
</body>
</html>
    "###;

    Html(html.to_string())
}

pub async fn chat_interface_with_session(
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> Html<String> {
    // Pass the session ID to the chat interface
    chat_interface_with_session_id(Some(session_id)).await
}

pub async fn chat_interface() -> Html<String> {
    chat_interface_with_session_id(None).await
}

pub async fn chat_interface_with_session_id(session_id: Option<String>) -> Html<String> {
    let session_id_js = match session_id {
        Some(id) => format!("'{}'", id),
        None => "null".to_string(),
    };

    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>🎬 VideoSync</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        html {
            scroll-behavior: smooth;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            background-size: cover;
            background-position: center;
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
            height: 100vh;
            overflow: hidden;
        }

        .app-container {
            display: flex;
            height: 100vh;
            max-width: 1400px;
            margin: 0 auto;
            background: rgba(255, 255, 255, 0.95);
            backdrop-filter: blur(10px);
            box-shadow: 0 0 50px rgba(0,0,0,0.1);
        }

        /* Sidebar */
        .sidebar {
            width: 300px;
            background: #2c3e50;
            color: white;
            display: flex;
            flex-direction: column;
            border-right: 1px solid #34495e;
        }

        .sidebar-header {
            padding: 20px;
            background: #1a252f;
            border-bottom: 1px solid #34495e;
        }

        .sidebar-header h1 {
            font-size: 1.5rem;
            margin-bottom: 0.5rem;
        }

        .sidebar-header p {
            color: #bdc3c7;
            font-size: 0.9rem;
        }

        .file-manager {
            flex: 1;
            padding: 20px;
            overflow-y: auto;
        }

        .file-manager h3 {
            margin-bottom: 15px;
            color: #ecf0f1;
            font-size: 1rem;
        }

        .file-list {
            space-y: 8px;
        }

        .file-item {
            background: #34495e;
            padding: 12px;
            border-radius: 8px;
            margin-bottom: 8px;
            cursor: pointer;
            transition: background-color 0.2s;
        }

        .file-item:hover {
            background: #3b4f61;
        }

        .file-name {
            font-weight: 500;
            font-size: 0.9rem;
            margin-bottom: 4px;
        }

        .file-meta {
            font-size: 0.8rem;
            color: #95a5a6;
        }

        .upload-btn {
            margin: 20px;
            padding: 12px;
            background: #3498db;
            color: white;
            border: none;
            border-radius: 8px;
            cursor: pointer;
            font-weight: 500;
            transition: background-color 0.2s;
        }

        .upload-btn:hover {
            background: #2980b9;
        }

        /* Main Content */
        .main-content {
            flex: 1;
            display: flex;
            flex-direction: column;
        }

        .chat-header {
            padding: 20px;
            background: #f8f9fa;
            border-bottom: 1px solid #e9ecef;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .chat-title {
            font-size: 1.2rem;
            font-weight: 600;
            color: #2c3e50;
        }

        .status-indicator {
            display: flex;
            align-items: center;
            gap: 8px;
            font-size: 0.9rem;
            color: #6c757d;
        }

        .status-dot {
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background: #28a745;
        }

        .status-dot.disconnected {
            background: #dc3545;
        }

        /* Chat Area */
        .chat-container {
            flex: 1;
            display: flex;
            flex-direction: column;
            overflow: hidden;
            position: relative; /* Ensure absolute positioned children stay inside */
        }

        .chat-messages {
            flex: 1;
            padding: 20px;
            overflow-y: auto;
            background: #ffffff;
            scroll-behavior: smooth;
        }

        .message {
            margin-bottom: 20px;
            display: flex;
            align-items: flex-start;
            gap: 12px;
        }

        .message.user {
            flex-direction: row-reverse;
        }

        .message-avatar {
            width: 40px;
            height: 40px;
            border-radius: 50%;
            display: flex;
            align-items: center;
            justify-content: center;
            font-weight: bold;
            color: white;
            font-size: 0.9rem;
        }

        .message.user .message-avatar {
            background: #3498db;
        }

        .message.assistant .message-avatar {
            background: #e74c3c;
        }

        .message-content {
            max-width: 70%;
            padding: 12px 16px;
            border-radius: 18px;
            line-height: 1.4;
        }

        .message.user .message-content {
            background: #3498db;
            color: white;
            border-bottom-right-radius: 4px;
        }

        .message.assistant .message-content {
            background: #f1f3f4;
            color: #2c3e50;
            border-bottom-left-radius: 4px;
        }

        .message-time {
            font-size: 0.8rem;
            color: #6c757d;
            margin-top: 4px;
        }

        /* Download and Stream Buttons */
        .download-button, .stream-button, .youtube-button {
            display: inline-block;
            margin: 10px 5px;
            padding: 10px 20px;
            color: white;
            text-decoration: none;
            border-radius: 25px;
            font-weight: 600;
            transition: transform 0.2s, box-shadow 0.2s;
            border: none;
            cursor: pointer;
        }

        .download-button {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
        }

        .stream-button {
            background: linear-gradient(135deg, #f093fb 0%, #f5576c 100%);
        }

        .youtube-button {
            background: linear-gradient(135deg, #FF0000 0%, #CC0000 100%);
        }

        .download-button:hover, .stream-button:hover, .youtube-button:hover {
            transform: translateY(-2px);
            box-shadow: 0 10px 20px rgba(102, 126, 234, 0.4);
        }

        /* Input Area */
        .chat-input-container {
            padding: 20px;
            background: #f8f9fa;
            border-top: 1px solid #e9ecef;
        }

        .chat-input-wrapper {
            display: flex;
            gap: 12px;
            align-items: flex-end;
        }

        .chat-input {
            flex: 1;
            min-height: 44px;
            max-height: 120px;
            padding: 12px 16px;
            border: 2px solid #e9ecef;
            border-radius: 22px;
            font-size: 1rem;
            resize: none;
            outline: none;
            transition: border-color 0.2s;
        }

        .chat-input:focus {
            border-color: #3498db;
        }

        .send-btn {
            width: 44px;
            height: 44px;
            border: none;
            border-radius: 50%;
            background: #3498db;
            color: white;
            cursor: pointer;
            display: flex;
            align-items: center;
            justify-content: center;
            transition: background-color 0.2s;
        }

        .send-btn:hover:not(:disabled) {
            background: #2980b9;
        }

        .send-btn:disabled {
            background: #bdc3c7;
            cursor: not-allowed;
        }

        /* Progress Bar */
        .progress-container {
            position: absolute; /* Changed from fixed to absolute to stay within chat-container */
            bottom: 100px; /* Positioned above the chat input */
            left: 50%;
            transform: translateX(-50%);
            width: 80%; /* Reduced width to prevent touching edges */
            max-width: 500px;
            background: rgba(26, 26, 46, 0.95);
            border-radius: 15px;
            padding: 15px;
            box-shadow: 0 10px 30px rgba(0, 0, 0, 0.3);
            backdrop-filter: blur(10px);
            display: none;
            z-index: 100; /* Lower z-index needed since it's inside container */
        }
        
        .progress-container.show {
            display: block;
            animation: slideUp 0.3s ease;
        }
        
        @keyframes slideUp {
            from {
                opacity: 0;
                transform: translateX(-50%) translateY(20px);
            }
            to {
                opacity: 1;
                transform: translateX(-50%) translateY(0);
            }
        }
        
        .progress-title {
            color: white;
            font-weight: 600;
            margin-bottom: 10px;
            display: flex;
            align-items: center;
            gap: 10px;
        }
        
        .progress-bar-outer {
            width: 100%;
            height: 8px;
            background: rgba(255, 255, 255, 0.1);
            border-radius: 10px;
            overflow: hidden;
            margin-bottom: 10px;
        }
        
        .progress-bar-inner {
            height: 100%;
            background: linear-gradient(90deg, #3498db, #667eea);
            border-radius: 10px;
            transition: width 0.3s ease;
            position: relative;
            overflow: hidden;
        }
        
        .progress-bar-inner::after {
            content: '';
            position: absolute;
            top: 0;
            left: 0;
            bottom: 0;
            right: 0;
            background: linear-gradient(
                90deg,
                transparent,
                rgba(255, 255, 255, 0.3),
                transparent
            );
            animation: shimmer 2s infinite;
        }
        
        @keyframes shimmer {
            0% {
                transform: translateX(-100%);
            }
            100% {
                transform: translateX(100%);
            }
        }
        
        .progress-text {
            color: #95a5a6;
            font-size: 14px;
        }
        
        /* Tool Execution Display */
        .tool-execution {
            background: rgba(52, 152, 219, 0.1);
            border-left: 4px solid #3498db;
            padding: 10px 15px;
            margin: 10px 0;
            border-radius: 5px;
            animation: fadeIn 0.3s ease;
        }
        
        .tool-execution-title {
            color: #3498db;
            font-weight: 600;
            margin-bottom: 5px;
        }
        
        .tool-execution-details {
            color: #95a5a6;
            font-size: 14px;
        }
        
        /* Welcome Screen */
        .welcome-screen {
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            height: 100%;
            text-align: center;
            color: #6c757d;
        }

        .welcome-screen h2 {
            font-size: 1.5rem;
            margin-bottom: 1rem;
            color: #2c3e50;
        }

        .welcome-screen p {
            max-width: 400px;
            line-height: 1.6;
            margin-bottom: 2rem;
        }

        .example-prompts {
            display: flex;
            flex-direction: column;
            gap: 8px;
        }

        .example-prompt {
            padding: 8px 16px;
            background: #f8f9fa;
            border: 1px solid #e9ecef;
            border-radius: 20px;
            cursor: pointer;
            transition: all 0.2s;
            font-size: 0.9rem;
        }

        .example-prompt:hover {
            background: #e9ecef;
            transform: translateY(-2px);
            box-shadow: 0 4px 12px rgba(0,0,0,0.1);
        }

        /* Loading animation */
        .typing-indicator {
            display: none;
            align-items: center;
            gap: 8px;
            padding: 12px 16px;
            background: #f1f3f4;
            border-radius: 18px;
            margin-bottom: 20px;
        }

        .typing-indicator.show {
            display: flex;
        }

        .typing-dots {
            display: flex;
            gap: 4px;
        }

        .typing-dot {
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background: #6c757d;
            animation: typing 1.4s infinite;
        }

        .typing-dot:nth-child(2) {
            animation-delay: 0.2s;
        }

        .typing-dot:nth-child(3) {
            animation-delay: 0.4s;
        }

        @keyframes typing {
            0%, 60%, 100% {
                transform: translateY(0);
                opacity: 0.4;
            }
            30% {
                transform: translateY(-10px);
                opacity: 1;
            }
        }
    </style>
</head>
<body>
    <div class="app-container">
        <!-- Sidebar -->
        <div class="sidebar">
            <div class="sidebar-header">
                <h1>🎬 Video Editor</h1>
                <p>AI-powered video editing</p>
            </div>
            
            <div class="file-manager">
                <h3>📁 Uploaded Files</h3>
                <div id="fileList" class="file-list">
                    <div class="file-item" style="opacity: 0.5;">
                        <div class="file-name">No files uploaded yet</div>
                        <div class="file-meta">Upload files to get started</div>
                    </div>
                </div>
            </div>
            
            <button class="upload-btn" onclick="uploadFiles()">
                📤 Upload Files
            </button>
        </div>

        <!-- Main Content -->
        <div class="main-content">
            <div class="chat-header">
                <div>
                    <div class="chat-title">Video Editing Assistant</div>
                    <div style="display:flex;gap:8px;flex-wrap:wrap;margin-top:8px;">
                        <a href="/services" style="text-decoration:none;padding:6px 10px;border-radius:999px;background:rgba(59,130,246,0.18);border:1px solid rgba(96,165,250,0.28);color:#dbeafe;font-size:12px;font-weight:600;">Services</a>
                        <a href="/services/saas-launch-pack" style="text-decoration:none;padding:6px 10px;border-radius:999px;background:rgba(15,23,42,0.72);border:1px solid rgba(148,163,184,0.18);color:#dbeafe;font-size:12px;">SaaS Launch</a>
                        <a href="/services/clipper-enhancement-pack" style="text-decoration:none;padding:6px 10px;border-radius:999px;background:rgba(15,23,42,0.72);border:1px solid rgba(148,163,184,0.18);color:#dbeafe;font-size:12px;">Motion Pack</a>
                        <a href="/services/creator-manager-fulfillment" style="text-decoration:none;padding:6px 10px;border-radius:999px;background:rgba(15,23,42,0.72);border:1px solid rgba(148,163,184,0.18);color:#dbeafe;font-size:12px;">Agency Backend</a>
                        <a href="/services/x402-asset-api" style="text-decoration:none;padding:6px 10px;border-radius:999px;background:rgba(15,23,42,0.72);border:1px solid rgba(148,163,184,0.18);color:#dbeafe;font-size:12px;">Payments</a>
                    </div>
                </div>
                <div style="display: flex; gap: 15px; align-items: center;">
                    <div class="status-indicator">
                        <div id="statusDot" class="status-dot disconnected"></div>
                        <span id="statusText">Connecting...</span>
                    </div>
                </div>
            </div>

            <div class="chat-container">
                <div id="chatMessages" class="chat-messages">
                    <div class="welcome-screen">
                        <h2>Welcome to your AI Video Editor! 🎬</h2>
                        <p>I can help you edit videos using natural language. Upload your files and tell me what you'd like to do!</p>
                        
                        <div class="example-prompts">
                            <div class="example-prompt" onclick="sendExamplePrompt('Trim my video from 10 seconds to 30 seconds')">
                                "Trim my video from 10 seconds to 30 seconds"
                            </div>
                            <div class="example-prompt" onclick="sendExamplePrompt('Add text overlay saying Hello World')">
                                "Add text overlay saying 'Hello World'"
                            </div>
                            <div class="example-prompt" onclick="sendExamplePrompt('Convert my video to MP4 format')">
                                "Convert my video to MP4 format"
                            </div>
                            <div class="example-prompt" onclick="sendExamplePrompt('Analyze my video and tell me its properties')">
                                "Analyze my video and tell me its properties"
                            </div>
                        </div>
                    </div>
                </div>

                <div class="typing-indicator" id="typingIndicator">
                    <div class="message-avatar" style="background: #e74c3c;">🤖</div>
                    <div style="display: flex; align-items: center; gap: 8px;">
                        <span id="thinkingText">AI is thinking</span>
                        <div class="typing-dots">
                            <div class="typing-dot"></div>
                            <div class="typing-dot"></div>
                            <div class="typing-dot"></div>
                        </div>
                    </div>
                </div>
                
                <!-- Progress Bar Container -->
                <div class="progress-container" id="progressContainer">
                    <div class="progress-title">
                        <span>🎬</span>
                        <span id="progressTitle">Processing video...</span>
                    </div>
                    <div class="progress-bar-outer">
                        <div class="progress-bar-inner" id="progressBar" style="width: 0%"></div>
                    </div>
                    <div class="progress-text" id="progressText">Initializing...</div>
                </div>

                <div class="chat-input-container">
                    <div class="chat-input-wrapper">
                        <textarea 
                            id="chatInput" 
                            class="chat-input" 
                            placeholder="Ask me to edit your videos... (e.g., 'trim my video from 10s to 30s')"
                            rows="1"
                        ></textarea>
                        <button id="sendBtn" class="send-btn" onclick="sendMessage()">
                            <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
                                <path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z"/>
                            </svg>
                        </button>
                    </div>
                </div>
            </div>
        </div>
    </div>

    <!-- Hidden file input -->
    <input type="file" id="fileInput" multiple accept="video/*,audio/*,image/*,.pdf,.doc,.docx,.txt" style="display: none;">

    <!-- YouTube Upload Modal -->
    <div id="youtubeModal" style="display: none; position: fixed; top: 0; left: 0; width: 100%; height: 100%; background: rgba(0,0,0,0.7); z-index: 10000; justify-content: center; align-items: center;">
        <div style="background: white; border-radius: 15px; padding: 2rem; max-width: 600px; width: 90%; max-height: 80vh; overflow-y: auto;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem;">
                <h2 style="color: #2c3e50; margin: 0;">📺 Post to YouTube</h2>
                <button onclick="closeYouTubeModal()" style="background: none; border: none; font-size: 1.5rem; cursor: pointer; color: #6c757d;">×</button>
            </div>

            <div id="youtubeModalContent">
                <p style="text-align: center; padding: 2rem; color: #6c757d;">Loading your channels...</p>
            </div>
        </div>
    </div>

    <script>
        console.log('✅ SCRIPT TAG LOADED - JavaScript is executing');

        let ws = null;
        let isConnected = false;
        let uploadedFiles = [];
        let jobPollTimer = null;
        let seenJobIds = new Set();
        // Session ID passed from server (either specific session or null for new)
        let providedSessionId = SESSION_ID_PLACEHOLDER;

        console.log('📝 Provided Session ID:', providedSessionId);

        let sessionUuid = providedSessionId || generateUUID();

        console.log('🆔 Final Session UUID:', sessionUuid);

        // Initialize the application
        document.addEventListener('DOMContentLoaded', function() {
            console.log('🚀 DOMContentLoaded event fired - initializing chat interface');
            console.log('Auth token present:', !!localStorage.getItem('auth_token') || localStorage.getItem('authToken'));

            try {
                initializeSession();
                initializeWebSocket();
                setupEventListeners();
                loadUploadedFiles();
                // Load chat history if we have an existing session
                if (providedSessionId) {
                    loadChatHistory(providedSessionId);
                }
                applyPrefilledPromptFromQuery();
            } catch (error) {
                console.error('❌ FATAL: Error during initialization:', error);
                alert('Failed to initialize chat: ' + error.message);
            }
        });

        function applyPrefilledPromptFromQuery() {
            const params = new URLSearchParams(window.location.search);
            const prompt = params.get('prompt');
            const autosend = params.get('autosend') === '1';
            const sampleRequestId = params.get('sample_request_id');
            if (!prompt) return;

            const input = document.getElementById('chatInput');
            if (!input) return;

            input.value = prompt;
            input.style.height = 'auto';
            input.style.height = Math.min(input.scrollHeight, 120) + 'px';

            if (!autosend) return;

            const autosendKey = sampleRequestId
                ? `videosync:autosent:${sampleRequestId}`
                : `videosync:autosent:${sessionUuid}:${prompt}`;
            if (sessionStorage.getItem(autosendKey) === '1') return;

            const trySend = () => {
                if (!isConnected) {
                    setTimeout(trySend, 500);
                    return;
                }
                const pendingPrompt = input.value.trim();
                if (!pendingPrompt) return;
                sessionStorage.setItem(autosendKey, '1');
                sendMessage();
                const cleanParams = new URLSearchParams(window.location.search);
                cleanParams.delete('prompt');
                cleanParams.delete('autosend');
                cleanParams.delete('sample_request_id');
                const cleanQuery = cleanParams.toString();
                const nextUrl = cleanQuery ? `${window.location.pathname}?${cleanQuery}` : window.location.pathname;
                window.history.replaceState({}, '', nextUrl);
            };

            setTimeout(trySend, 300);
        }

        function generateUUID() {
            return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function(c) {
                var r = Math.random() * 16 | 0, v = c == 'x' ? r : (r & 0x3 | 0x8);
                return v.toString(16);
            });
        }

        function initializeSession() {
            console.log('Session UUID:', sessionUuid);
            if (providedSessionId) {
                console.log('Loading existing session:', providedSessionId);
            } else {
                console.log('Starting new session');
            }
        }
        
        async function loadChatHistory(sessionId) {
            try {
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
                if (!authToken) {
                    console.warn('No auth token, cannot load chat history');
                    return;
                }
                
                const response = await fetch(`/api/chat/history/${sessionId}`, {
                    headers: {
                        'Authorization': `Bearer ${authToken}`
                    }
                });
                
                if (response.ok) {
                    const data = await response.json();
                    if (data.success && data.history && data.history.length > 0) {
                        // Clear welcome screen
                        const messagesContainer = document.getElementById('chatMessages');
                        const welcomeScreen = messagesContainer.querySelector('.welcome-screen');
                        if (welcomeScreen) {
                            welcomeScreen.style.display = 'none';
                        }
                        
                        // Add historical messages with actual timestamps
                        console.log('History data:', data.history);
                        data.history.forEach(msg => {
                            // Each history item has user_message, agent_response, and timestamp
                            // The timestamp represents when the user sent the message
                            if (msg.user_message && msg.user_message.trim() !== '') {
                                addMessage('user', msg.user_message, false, msg.timestamp);
                            }
                            if (msg.agent_response && msg.agent_response.trim() !== '') {
                                // Use same timestamp for the response (it came right after)
                                addMessage('assistant', msg.agent_response, false, msg.timestamp);
                            }
                        });
                        
                        // Scroll to bottom after loading all messages
                        messagesContainer.scrollTop = messagesContainer.scrollHeight;
                        
                        console.log(`Loaded ${data.history.length} historical messages`);
                    }
                } else {
                    const errorData = await response.json();
                    if (errorData.message && errorData.message.includes('Access denied')) {
                        console.error('Access denied to this chat session');
                        addMessage('assistant', "⚠️ You don't have permission to view this chat session.");
                        // Redirect to new chat after 2 seconds
                        setTimeout(() => {
                            window.location.href = '/chat';
                        }, 2000);
                    } else {
                        console.error('Failed to load chat history');
                    }
                }
            } catch (error) {
                console.error('Error loading chat history:', error);
            }
        }

        function initializeWebSocket() {
            try {
                const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
                const tokenParam = authToken ? '&token=' + encodeURIComponent(authToken) : '';
                const wsUrl = `${protocol}//${window.location.host}/ws?session=${sessionUuid}${tokenParam}`;

                console.log('Attempting WebSocket connection to:', wsUrl);
                console.log('Session UUID:', sessionUuid);

                ws = new WebSocket(wsUrl);

                ws.onopen = function() {
                    isConnected = true;
                    updateConnectionStatus(true);
                    stopJobPolling(); // WS live — polling not needed
                    console.log('✅ Successfully connected to video editing assistant');
                };

            ws.onmessage = function(event) {
                const data = event.data;
                
                try {
                    const jsonData = JSON.parse(data);
                    
                    console.log('Received message:', jsonData.type, jsonData.content?.substring(0, 100));

                    switch (jsonData.type) {
                        case 'message':
                            hideTypingIndicator();
                            hideProgressBar();
                            seenJobIds.add('ws-' + Date.now());
                            addMessage('assistant', jsonData.content);
                            break;
                        case 'thinking':
                            // Update typing indicator with agent thinking/tool calling details
                            console.log('Thinking update:', jsonData.content);
                            updateThinkingIndicator(jsonData.content);
                            break;
                        case 'background_job_status':
                            // Shown on reconnect when a workflow has a persisted progress step
                            hideTypingIndicator();
                            addMessage('assistant', jsonData.content, true /* isStatus */);
                            break;
                        case 'progress':
                            // Show agent progress in typing indicator, background job progress in progress bar
                            if (jsonData.content.includes('🤖') || jsonData.content.includes('🔧') ||
                                jsonData.content.includes('🧠') || jsonData.content.includes('📚') ||
                                jsonData.content.includes('💾') || jsonData.content.includes('🔮')) {
                                // Agent-related progress - update typing indicator
                                updateThinkingIndicator(jsonData.content);
                            } else {
                                // Background job progress - show in progress bar
                                updateProgressBar(jsonData);
                            }
                            break;
                        case 'tool_call':
                            showToolExecution(jsonData.details);
                            break;
                        default:
                            // Fallback for unknown JSON types
                            hideTypingIndicator();
                            addMessage('assistant', data);
                    }
                } catch (error) {
                    // Data is not JSON, treat as plain text
                    if (data.startsWith('PROGRESS:')) {
                        const progressInfo = data.substring(9);
                        // This is a legacy format, we should adapt it to the new progress bar
                        const parts = progressInfo.split('|');
                        const progressData = {
                            status: {
                                progress_percent: parseInt(parts[0]) || 0,
                                current_step: parts[1] || '',
                            },
                            message: parts[2] || parts[1] || '',
                        };
                        updateProgressBar(progressData);
                    } else if (data.startsWith('TOOL_CALL:')) {
                        const toolInfo = data.substring(10);
                        showToolExecution(toolInfo);
                    } else {
                        hideTypingIndicator();
                        hideProgressBar();
                        addMessage('assistant', data);
                    }
                }
            };
            
            ws.onclose = function() {
                isConnected = false;
                updateConnectionStatus(false);
                console.log('Disconnected from assistant');

                // Start polling for completed background jobs while WS is down
                startJobPolling();

                // Try to reconnect after 3 seconds
                setTimeout(initializeWebSocket, 3000);
            };
            
            ws.onerror = function(error) {
                console.error('❌ WebSocket error:', error);
                console.error('Error details:', {
                    type: error.type,
                    target: error.target?.url || 'unknown'
                });
                hideTypingIndicator();
                updateConnectionStatus(false);
            };
            } catch (error) {
                console.error('❌ Failed to initialize WebSocket:', error);
                updateConnectionStatus(false);
            }
        }

        // ─── Background job polling (SSR fallback when WS is disconnected) ──────
        function startJobPolling() {
            if (jobPollTimer) return;
            jobPollTimer = setInterval(async function() {
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
                if (!authToken) return;
                try {
                    const res = await fetch(`/api/chat/sessions/${sessionUuid}/jobs`, {
                        headers: { 'Authorization': 'Bearer ' + authToken }
                    });
                    if (!res.ok) return;
                    const data = await res.json();
                    const jobs = data.jobs || [];
                    for (const job of jobs) {
                        const progressLog = Array.isArray(job.progress_log) ? job.progress_log : [];
                        const latestProgress = progressLog.length ? progressLog[progressLog.length - 1] : null;
                        const latestProgressMessage = latestProgress && latestProgress.msg
                            ? String(latestProgress.msg)
                            : '';
                        const latestWorkflowEventMessage = job.workflow && job.workflow.latest_event_message
                            ? String(job.workflow.latest_event_message)
                            : '';
                        const workflowStep = job.workflow && job.workflow.current_step
                            ? String(job.workflow.current_step)
                            : '';

                        if (job.status === 'completed' && job.result) {
                            const msgId = 'job-' + job.id;
                            if (!seenJobIds.has(msgId)) {
                                seenJobIds.add(msgId);
                                hideTypingIndicator();
                                addMessage('assistant', job.result);
                            }
                        } else if (job.status === 'failed' && job.error) {
                            const msgId = 'job-err-' + job.id;
                            if (!seenJobIds.has(msgId)) {
                                seenJobIds.add(msgId);
                                hideTypingIndicator();
                                addMessage('assistant', job.error);
                            }
                        } else if (job.status === 'running') {
                            if (latestProgressMessage) {
                                updateThinkingIndicator(latestProgressMessage);
                            } else if (latestWorkflowEventMessage) {
                                updateThinkingIndicator(latestWorkflowEventMessage);
                            } else if (workflowStep) {
                                updateThinkingIndicator(workflowStep);
                            }
                        }
                    }
                } catch(e) {
                    console.warn('Job polling error:', e);
                }
            }, 10000); // every 10s
        }

        function stopJobPolling() {
            if (jobPollTimer) {
                clearInterval(jobPollTimer);
                jobPollTimer = null;
            }
        }
        // ─────────────────────────────────────────────────────────────────────

        function setupEventListeners() {
            const chatInput = document.getElementById('chatInput');
            const sendBtn = document.getElementById('sendBtn');
            
            // Auto-resize textarea
            chatInput.addEventListener('input', function() {
                this.style.height = 'auto';
                this.style.height = Math.min(this.scrollHeight, 120) + 'px';
            });
            
            // Send on Enter, new line on Shift+Enter
            chatInput.addEventListener('keydown', function(e) {
                if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault();
                    sendMessage();
                }
            });
            
            // File input handler
            document.getElementById('fileInput').addEventListener('change', handleFileUpload);
        }

        function updateConnectionStatus(connected) {
            const statusDot = document.getElementById('statusDot');
            const statusText = document.getElementById('statusText');
            
            if (connected) {
                statusDot.classList.remove('disconnected');
                statusText.textContent = 'Connected';
            } else {
                statusDot.classList.add('disconnected');
                statusText.textContent = 'Disconnected';
            }
        }

        function sendMessage() {
            const input = document.getElementById('chatInput');
            const message = input.value.trim();
            
            if (!message || !isConnected) return;
            
            // Add user message to chat
            addMessage('user', message);
            
            // Clear input
            input.value = '';
            input.style.height = 'auto';
            
            // Show typing indicator
            showTypingIndicator();
            
            // Send to WebSocket
            ws.send(message);
        }

        function sendExamplePrompt(prompt) {
            document.getElementById('chatInput').value = prompt;
            sendMessage();
        }

        function extractAssistantText(content) {
            if (typeof content !== 'string') return content;
            const trimmed = content.trim();
            if (!trimmed.startsWith('{')) return content;

            try {
                const parsed = JSON.parse(trimmed);
                if (parsed && typeof parsed.content === 'string') {
                    return parsed.content;
                }
            } catch (_) {
                return content;
            }

            return content;
        }

        function addMessage(sender, content, shouldScroll = true, timestamp = null) {
            const messagesContainer = document.getElementById('chatMessages');
            const welcomeScreen = messagesContainer.querySelector('.welcome-screen');

            // Hide welcome screen on first message
            if (welcomeScreen) {
                welcomeScreen.style.display = 'none';
            }

            const messageDiv = document.createElement('div');
            messageDiv.className = `message ${sender}`;

            // Use provided timestamp or current time
            const messageTime = timestamp ? new Date(timestamp) : new Date();
            const timeString = messageTime.toLocaleTimeString([], {hour: '2-digit', minute:'2-digit'});
            
            // Process content to add download links if it's from assistant
            let processedContent = content;
            if (sender === 'assistant') {
                processedContent = parseAndRenderDownloadLinks(extractAssistantText(content));
            }
            
            messageDiv.innerHTML = `
                <div class="message-avatar">
                    ${sender === 'user' ? '👤' : '🤖'}
                </div>
                <div class="message-content">
                    ${processedContent}
                    <div class="message-time">${timeString}</div>
                </div>
            `;
            
            messagesContainer.appendChild(messageDiv);
            
            // Only scroll if requested (not when loading history)
            if (shouldScroll) {
                messagesContainer.scrollTop = messagesContainer.scrollHeight;
            }
        }
        
        function parseAndRenderDownloadLinks(content) {
            console.log('Original content:', content);

            // Parse download/stream/YouTube URLs and create clickable buttons
            const downloadRegex = /Download:\s*`([^`]+)`/g;
            const streamRegex = /Stream:\s*`([^`]+)`/g;
            const youtubeRegex = /YouTube:\s*`([^|]+)\|([^`]+)`/g;
            const fileNameRegex = /\*\*([^*]+\.mp4)\*\*/g;

            let processedContent = content;
            let fileName = '';

            // Extract filename from markdown bold syntax
            const fileMatch = fileNameRegex.exec(content);
            if (fileMatch) {
                fileName = fileMatch[1].trim();
                console.log('Extracted filename:', fileName);
            }

            // Replace download links with buttons FIRST (before converting newlines)
            processedContent = processedContent.replace(downloadRegex, (match, url) => {
                console.log('Replacing download URL:', url);
                return `<a href="${url}" download="${fileName}" class="download-button">📥 Download Video</a>`;
            });

            // Replace stream links with buttons
            processedContent = processedContent.replace(streamRegex, (match, url) => {
                console.log('Replacing stream URL:', url);
                return `<a href="${url}" target="_blank" class="stream-button">▶️ Stream Video</a>`;
            });

            // Replace YouTube links with buttons that open channel selector
            processedContent = processedContent.replace(youtubeRegex, (match, videoPath, videoName) => {
                console.log('Replacing YouTube link:', videoPath, videoName);
                return `<button onclick="openYouTubeUploadModal('${videoPath}', '${videoName}')" class="youtube-button">📺 Post to YouTube</button>`;
            });

            // Also handle the case where URLs are shown directly (not in backticks)
            processedContent = processedContent.replace(/Download:\s*(\/api\/outputs\/download\/[a-f0-9]+)/gi, (match, url) => {
                console.log('Replacing direct download URL:', url);
                return `<a href="${url}" download="${fileName}" class="download-button">📥 Download Video</a>`;
            });

            processedContent = processedContent.replace(/Stream:\s*(\/api\/outputs\/stream\/[a-f0-9]+)/gi, (match, url) => {
                console.log('Replacing direct stream URL:', url);
                return `<a href="${url}" target="_blank" class="stream-button">▶️ Stream Video</a>`;
            });

            // Format the content better with proper line breaks and styling
            processedContent = processedContent
                .replace(/\*\*(.*?)\*\*/g, '<strong>$1</strong>')
                .replace(/•/g, '<br>•')
                .replace(/\n/g, '<br>');

            console.log('Processed content:', processedContent);
            return processedContent;
        }

        // ============================================================================
        // YouTube Upload Modal Functions
        // ============================================================================

        let currentYouTubeVideo = null;

        async function openYouTubeUploadModal(videoPath, videoName) {
            currentYouTubeVideo = { path: videoPath, name: videoName };
            const modal = document.getElementById('youtubeModal');
            const content = document.getElementById('youtubeModalContent');

            modal.style.display = 'flex';
            content.innerHTML = '<p style="text-align: center; padding: 2rem; color: #6c757d;">Loading your channels...</p>';

            try {
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
                if (!authToken) {
                    content.innerHTML = `
                        <div style="text-align: center; padding: 2rem;">
                            <p style="color: #dc3545; margin-bottom: 1rem;">Please log in to upload to YouTube</p>
                            <button onclick="window.location.href='/login'" class="btn">Go to Login</button>
                        </div>
                    `;
                    return;
                }

                const response = await fetch('/api/youtube/channels', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();

                if (data.success && data.channels.length > 0) {
                    content.innerHTML = `
                        <div style="margin-bottom: 1.5rem;">
                            <h3 style="color: #2c3e50; margin-bottom: 1rem;">Select a channel:</h3>
                            <div id="channelList" style="display: flex; flex-direction: column; gap: 1rem;">
                                ${data.channels.map(channel => `
                                    <div onclick="selectYouTubeChannel(${channel.id}, '${channel.channel_name}')"
                                         style="display: flex; align-items: center; gap: 1rem; padding: 1rem; border: 2px solid #e9ecef; border-radius: 10px; cursor: pointer; transition: all 0.2s;"
                                         onmouseover="this.style.borderColor='#3b82f6'; this.style.background='#f8f9fa'"
                                         onmouseout="this.style.borderColor='#e9ecef'; this.style.background='white'">
                                        ${channel.channel_thumbnail_url ?
                                            `<img src="${channel.channel_thumbnail_url}" style="width: 40px; height: 40px; border-radius: 50%;" alt="${channel.channel_name}">` :
                                            '<div style="width: 40px; height: 40px; border-radius: 50%; background: linear-gradient(135deg, #FF0000, #CC0000); display: flex; align-items: center; justify-content: center; color: white; font-size: 1.2rem;">📺</div>'
                                        }
                                        <div style="flex: 1;">
                                            <div style="font-weight: 600; color: #2c3e50;">${channel.channel_name}</div>
                                            <div style="font-size: 0.85rem; color: #6c757d;">
                                                ${channel.subscriber_count !== null ? channel.subscriber_count.toLocaleString() + ' subscribers' : ''}
                                            </div>
                                        </div>
                                    </div>
                                `).join('')}
                            </div>
                        </div>
                        <div style="text-align: center; padding-top: 1rem; border-top: 1px solid #e9ecef;">
                            <a href="/youtube/manage" style="color: #3b82f6; text-decoration: none; font-weight: 500;">Manage Channels</a>
                        </div>
                    `;
                } else {
                    content.innerHTML = `
                        <div style="text-align: center; padding: 2rem;">
                            <div style="font-size: 3rem; margin-bottom: 1rem;">📺</div>
                            <h3 style="color: #2c3e50; margin-bottom: 1rem;">No YouTube Channels Connected</h3>
                            <p style="color: #6c757d; margin-bottom: 1.5rem;">Connect your YouTube channel to start uploading videos directly</p>
                            <button onclick="connectYouTubeChannel()" class="btn">Connect YouTube Channel</button>
                        </div>
                    `;
                }
            } catch (error) {
                console.error('Error loading channels:', error);
                content.innerHTML = `
                    <div style="text-align: center; padding: 2rem;">
                        <p style="color: #dc3545; margin-bottom: 1rem;">❌ Error loading channels</p>
                        <p style="color: #6c757d;">${error.message}</p>
                    </div>
                `;
            }
        }

        function closeYouTubeModal() {
            document.getElementById('youtubeModal').style.display = 'none';
            currentYouTubeVideo = null;
        }

        async function connectYouTubeChannel() {
            try {
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
                if (!authToken) {
                    alert('Please log in first');
                    window.location.href = '/login';
                    return;
                }

                // Fetch OAuth URL from backend with Authorization header
                const response = await fetch('/youtube/connect?redirect_to=' + encodeURIComponent(window.location.pathname), {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });

                if (!response.ok) {
                    const error = await response.json();
                    alert('Failed to connect: ' + (error.message || 'Unknown error'));
                    return;
                }

                const data = await response.json();
                if (data.success && data.auth_url) {
                    // Redirect to Google OAuth page
                    window.location.href = data.auth_url;
                } else {
                    alert('Failed to get authorization URL');
                }
            } catch (error) {
                console.error('Error connecting YouTube:', error);
                alert('Failed to connect YouTube channel. Please try again.');
            }
        }

        async function selectYouTubeChannel(channelId, channelName) {
            if (!currentYouTubeVideo) {
                alert('No video selected');
                return;
            }

            const title = prompt("Enter title for your YouTube video:", currentYouTubeVideo.name.replace('.mp4', ''));
            if (!title) return;

            const description = prompt('Enter description (optional):', 'Created with VideoSync');
            const privacyStatus = prompt('Privacy status (public/private/unlisted):', 'private');

            if (!['public', 'private', 'unlisted'].includes(privacyStatus.toLowerCase())) {
                alert('Invalid privacy status. Using "private"');
            }

            const modal = document.getElementById('youtubeModalContent');
            modal.innerHTML = `
                <div style="text-align: center; padding: 2rem;">
                    <div style="font-size: 3rem; margin-bottom: 1rem;">📤</div>
                    <p style="color: #2c3e50; font-weight: 600; margin-bottom: 0.5rem;">Uploading to YouTube...</p>
                    <p style="color: #6c757d;">This may take a few moments</p>
                </div>
            `;

            try {
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
                const response = await fetch('/api/youtube/upload', {
                    method: 'POST',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({
                        channel_id: channelId,
                        video_path: currentYouTubeVideo.path,
                        title: title,
                        description: description || 'Created with VideoSync',
                        privacy_status: privacyStatus.toLowerCase(),
                        category: '22',
                        tags: ['AI', 'Video Editing', 'VideoSync']
                    })
                });

                const data = await response.json();

                if (data.success) {
                    modal.innerHTML = `
                        <div style="text-align: center; padding: 2rem;">
                            <div style="font-size: 3rem; margin-bottom: 1rem;">✅</div>
                            <h3 style="color: #28a745; margin-bottom: 1rem;">Upload Successful!</h3>
                            <p style="color: #2c3e50; margin-bottom: 1.5rem;">Your video has been uploaded to <strong>${channelName}</strong></p>
                            <a href="${data.upload.youtube_url}" target="_blank" style="display: inline-block; padding: 0.75rem 1.5rem; background: #FF0000; color: white; text-decoration: none; border-radius: 10px; font-weight: 600; margin-bottom: 1rem;">🎬 View on YouTube</a>
                            <br>
                            <button onclick="closeYouTubeModal()" style="padding: 0.5rem 1.5rem; background: #6c757d; color: white; border: none; border-radius: 10px; cursor: pointer;">Close</button>
                        </div>
                    `;
                } else {
                    modal.innerHTML = `
                        <div style="text-align: center; padding: 2rem;">
                            <div style="font-size: 3rem; margin-bottom: 1rem;">❌</div>
                            <h3 style="color: #dc3545; margin-bottom: 1rem;">Upload Failed</h3>
                            <p style="color: #6c757d; margin-bottom: 1.5rem;">${data.message}</p>
                            <button onclick="closeYouTubeModal()" style="padding: 0.75rem 1.5rem; background: #3b82f6; color: white; border: none; border-radius: 10px; cursor: pointer;">Close</button>
                        </div>
                    `;
                }
            } catch (error) {
                console.error('YouTube upload error:', error);
                modal.innerHTML = `
                    <div style="text-align: center; padding: 2rem;">
                        <div style="font-size: 3rem; margin-bottom: 1rem;">❌</div>
                        <h3 style="color: #dc3545; margin-bottom: 1rem;">Upload Error</h3>
                        <p style="color: #6c757d; margin-bottom: 1.5rem;">${error.message}</p>
                        <button onclick="closeYouTubeModal()" style="padding: 0.75rem 1.5rem; background: #3b82f6; color: white; border: none; border-radius: 10px; cursor: pointer;">Close</button>
                    </div>
                `;
            }
        }

        function showTypingIndicator() {
            document.getElementById('typingIndicator').classList.add('show');
            const messagesContainer = document.getElementById('chatMessages');
            messagesContainer.scrollTop = messagesContainer.scrollHeight;
        }

        function hideTypingIndicator() {
            document.getElementById('typingIndicator').classList.remove('show');
            // Reset to default text
            document.getElementById('thinkingText').textContent = 'AI is thinking';
        }

        function updateThinkingIndicator(message) {
            // Show the typing indicator if not already visible
            const indicator = document.getElementById('typingIndicator');
            if (!indicator.classList.contains('show')) {
                indicator.classList.add('show');
            }

            // Update the thinking text with real-time agent progress
            document.getElementById('thinkingText').textContent = message;

            // Auto-scroll to show the updated thinking message
            const messagesContainer = document.getElementById('chatMessages');
            messagesContainer.scrollTop = messagesContainer.scrollHeight;
        }

        function updateProgressBar(progressData) {
            const container = document.getElementById('progressContainer');
            const bar = document.getElementById('progressBar');
            const title = document.getElementById('progressTitle');
            const text = document.getElementById('progressText');

            console.log('Progress update:', progressData); // Debug

            // Check if job completed (lowercase due to serde rename_all)
            if (progressData.status && progressData.status.status === 'completed') {
                hideProgressBar();
                hideTypingIndicator();
                // Add completion message to chat
                const result = progressData.status.result || progressData.message;
                addMessage('assistant', result);
                return;
            }

            // Check if job failed (lowercase due to serde rename_all)
            if (progressData.status && progressData.status.status === 'failed') {
                hideProgressBar();
                hideTypingIndicator();
                const error = progressData.status.error || progressData.message;
                addMessage('assistant', '❌ ' + error);
                return;
            }

            // Running status
            if (progressData.status && progressData.status.status === 'running') {
                const percentage = progressData.status.progress_percent || 0;
                const progressTitle = progressData.status.current_step || 'Waiting for the next recorded production step';
                const progressDesc = progressData.message || 'The job is active, but no detailed progress message was attached to this update.';

            // Show progress container
            container.classList.add('show');

            // Update content
            title.textContent = progressTitle;
            text.textContent = `${progressDesc} (${percentage.toFixed(1)}%)`;
            bar.style.width = `${percentage}%`;

            // Hide after 100%
            if (percentage >= 100) {
                setTimeout(() => {
                    hideProgressBar();
                }, 3000);
            }
            }
        }

        function hideProgressBar() {
            const container = document.getElementById('progressContainer');
            container.classList.remove('show');
        }
        
        function showToolExecution(toolInfo) {
            const messagesContainer = document.getElementById('chatMessages');
            
            // Parse tool info (expected format: "toolName|parameters")
            const parts = toolInfo.split('|');
            const toolName = parts[0] || 'Processing';
            const parameters = parts[1] || '';
            
            const toolDiv = document.createElement('div');
            toolDiv.className = 'tool-execution';
            toolDiv.innerHTML = `
                <div class="tool-execution-title">⚡ Executing: ${toolName}</div>
                <div class="tool-execution-details">${parameters}</div>
            `;
            
            messagesContainer.appendChild(toolDiv);
            messagesContainer.scrollTop = messagesContainer.scrollHeight;
        }

        function uploadFiles() {
            document.getElementById('fileInput').click();
        }

        async function handleFileUpload(event) {
            const files = event.target.files;
            if (files.length === 0) return;
            
            // Get JWT token from localStorage
            const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
            if (!authToken) {
                addMessage('assistant', '❌ Please log in to upload files.');
                return;
            }
            
            const formData = new FormData();
            for (let file of files) {
                formData.append('files', file);
            }
            
            try {
                const response = await fetch(`/upload/session/${sessionUuid}`, {
                    method: 'POST',
                    headers: {
                        'Authorization': `Bearer ${authToken}`
                    },
                    body: formData
                });
                
                const result = await response.json();
                
                if (result.success) {
                    uploadedFiles = [...uploadedFiles, ...result.files];
                    updateFileList();
                    addMessage('assistant', `✅ Successfully uploaded ${result.files.length} files: ${result.files.map(f => f.original_name).join(', ')}`);
                } else {
                    addMessage('assistant', '❌ Failed to upload files. Please try again.');
                }
            } catch (error) {
                console.error('Upload error:', error);
                addMessage('assistant', '❌ Error uploading files: ' + error.message);
            }
            
            // Clear the file input
            event.target.value = '';
        }

        function updateFileList() {
            const fileList = document.getElementById('fileList');
            
            if (uploadedFiles.length === 0) {
                fileList.innerHTML = `
                    <div class="file-item" style="opacity: 0.5;">
                        <div class="file-name">No files uploaded yet</div>
                        <div class="file-meta">Upload files to get started</div>
                    </div>
                `;
                return;
            }
            
            fileList.innerHTML = uploadedFiles.map(file => `
                <div class="file-item" onclick="selectFile('${file.id}')">
                    <div class="file-name">${file.original_name}</div>
                    <div class="file-meta">${(file.file_size / 1024 / 1024).toFixed(2)} MB • ${file.file_type}</div>
                </div>
            `).join('');
        }

        function selectFile(fileId) {
            const file = uploadedFiles.find(f => f.id === fileId);
            if (file) {
                const input = document.getElementById('chatInput');
                input.value = `Please work with my file: ${file.original_name}`;
                input.focus();
            }
        }

        async function loadUploadedFiles() {
            // Get JWT token from localStorage
            const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
            if (!authToken) {
                return; // No auth token, can't load files
            }
            
            try {
                const response = await fetch(`/files/session/${sessionUuid}`, {
                    method: 'GET',
                    headers: {
                        'Authorization': `Bearer ${authToken}`
                    }
                });
                
                if (response.ok) {
                    const result = await response.json();
                    if (result.success && result.files) {
                        uploadedFiles = result.files;
                        updateFileList();
                    }
                }
            } catch (error) {
                console.error('Error loading uploaded files:', error);
            }
        }

        // Dynamic Background Management for Chat Interface
        class ChatDynamicBackgroundManager {
            constructor() {
                this.lastBackgroundUpdate = Date.now();
                this.updateInterval = 5 * 60 * 1000; // 5 minutes
                this.retryDelay = 30 * 1000; // 30 seconds on error
                this.isUpdating = false;
                
                this.init();
            }

            async init() {
                // Load initial background
                await this.updateBackground();
                
                // Set up periodic updates
                setInterval(() => {
                    this.checkAndUpdateBackground();
                }, 60 * 1000); // Check every minute
            }

            async checkAndUpdateBackground() {
                if (this.isUpdating) return;
                
                const timeSinceLastUpdate = Date.now() - this.lastBackgroundUpdate;
                if (timeSinceLastUpdate >= this.updateInterval) {
                    await this.updateBackground();
                }
            }

            async updateBackground() {
                if (this.isUpdating) return;
                
                this.isUpdating = true;
                
                try {
                    const response = await fetch('/api/background/image');
                    
                    if (response.ok) {
                        const contentType = response.headers.get('content-type');
                        
                        if (contentType && contentType.includes('application/json')) {
                            // Fallback gradient
                            const data = await response.json();
                            if (data.fallback && data.gradient) {
                                document.body.style.background = data.gradient;
                            }
                        } else {
                            // Image response
                            const blob = await response.blob();
                            const imageUrl = URL.createObjectURL(blob);
                            
                            // Create overlay for smooth transition
                            const overlay = document.createElement('div');
                            overlay.style.cssText = `
                                position: fixed;
                                top: 0;
                                left: 0;
                                width: 100%;
                                height: 100%;
                                background-image: url(${imageUrl});
                                background-size: cover;
                                background-position: center;
                                background-attachment: fixed;
                                opacity: 0;
                                transition: opacity 1s ease-in-out;
                                z-index: -1;
                                pointer-events: none;
                            `;
                            
                            document.body.appendChild(overlay);
                            
                            // Trigger fade in with moderate opacity for chat interface
                            setTimeout(() => {
                                overlay.style.opacity = '0.35'; // Visible but not distracting
                            }, 100);
                            
                            // Clean up old overlays after transition
                            setTimeout(() => {
                                const oldOverlays = document.querySelectorAll('div[style*="background-image"]');
                                oldOverlays.forEach((old, index) => {
                                    if (index < oldOverlays.length - 1) {
                                        old.remove();
                                    }
                                });
                            }, 1100);
                        }
                        
                        this.lastBackgroundUpdate = Date.now();
                    }
                } catch (error) {
                    console.error('Error updating chat background:', error);
                    setTimeout(() => {
                        this.lastBackgroundUpdate = Date.now() - this.updateInterval + this.retryDelay;
                    }, this.retryDelay);
                } finally {
                    this.isUpdating = false;
                }
            }
        }

        // Initialize dynamic background manager for chat
        document.addEventListener('DOMContentLoaded', () => {
            new ChatDynamicBackgroundManager();
        });
    </script>
</body>
</html>
    "###;

    // Replace the session ID placeholder with the actual value
    let html = html.replace("SESSION_ID_PLACEHOLDER", &session_id_js);

    Html(html)
}

// ============================================================================
// Analytics Dashboard Page (with dark theme + dynamic background)
// ============================================================================

pub async fn analytics_dashboard_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>📊 Analytics - VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-size: cover;
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
            min-height: 100vh;
            color: #e8e8e8;
            padding: 20px;
        }

        .container {
            max-width: 1400px;
            margin: 0 auto;
            background: rgba(26, 26, 46, 0.95);
            backdrop-filter: blur(20px);
            border-radius: 20px;
            padding: 40px;
            box-shadow: 0 20px 60px rgba(0,0,0,0.5);
            border: 1px solid rgba(59, 130, 246, 0.3);
        }

        .nav-link {
            color: #3b82f6;
            text-decoration: none;
            font-weight: 600;
            margin-bottom: 20px;
            display: inline-block;
        }

        h1 { color: #fff; font-size: 2.5rem; margin-bottom: 10px; }
        .subtitle { color: #bdc3c7; margin-bottom: 30px; }

        .info-box {
            background: rgba(59, 130, 246, 0.1);
            border-left: 4px solid #3b82f6;
            padding: 20px;
            border-radius: 10px;
            margin: 20px 0;
        }

        .info-box h3 { color: #fff; margin-bottom: 15px; }
        .info-box p { color: #bdc3c7; line-height: 1.6; margin-bottom: 10px; }
        .info-box ol { margin-left: 20px; margin-top: 10px; color: #bdc3c7; }

        .feature-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 20px;
            margin-top: 30px;
        }

        .feature-card {
            background: rgba(255,255,255,0.05);
            padding: 20px;
            border-radius: 12px;
            border: 1px solid rgba(59, 130, 246, 0.2);
            transition: transform 0.2s, border-color 0.2s;
        }

        .feature-card:hover {
            transform: translateY(-5px);
            border-color: rgba(59, 130, 246, 0.5);
        }

        .feature-card h4 { color: #3b82f6; margin-bottom: 10px; font-size: 1.2rem; }
        .feature-card ul { margin-left: 20px; margin-top: 10px; color: #bdc3c7; }

        .btn {
            display: inline-block;
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            padding: 12px 24px;
            border-radius: 25px;
            text-decoration: none;
            font-weight: 600;
            margin: 10px 5px;
            transition: all 0.3s;
            border: 1px solid rgba(59, 130, 246, 0.3);
        }

        .btn:hover {
            transform: translateY(-2px);
            box-shadow: 0 10px 20px rgba(59, 130, 246, 0.3);
        }

        .coming-soon {
            background: rgba(255, 193, 7, 0.1);
            border-left: 4px solid #ffc107;
            padding: 15px;
            border-radius: 8px;
            margin-top: 20px;
            color: #ffc107;
        }
    </style>
</head>
<body>
    <div class="container">
        <a href="/dashboard" class="nav-link">← Back to Dashboard</a>
        <h1>📊 Analytics Dashboard</h1>
        <p class="subtitle">YouTube Channel Performance & Video Analytics</p>

        <div class="info-box">
            <h3>🚀 Connect YouTube First</h3>
            <p>Connect your YouTube channel with analytics permissions to unlock performance insights.</p>
            <ol>
                <li>Click "Connect YouTube Channel"</li>
                <li>Grant <strong>YouTube Analytics</strong> permission</li>
                <li>Return here to view analytics</li>
            </ol>
            <a href="/youtube/connect" class="btn">📺 Connect YouTube Channel</a>
        </div>

        <div class="feature-grid">
            <div class="feature-card">
                <h4>📹 Video Analytics</h4>
                <ul>
                    <li>Views and watch time</li>
                    <li>Engagement metrics</li>
                    <li>Subscriber growth</li>
                    <li>Average view duration</li>
                </ul>
            </div>
            <div class="feature-card">
                <h4>📺 Channel Analytics</h4>
                <ul>
                    <li>Total views/subscribers</li>
                    <li>Revenue estimates</li>
                    <li>Demographics</li>
                    <li>Traffic sources</li>
                </ul>
            </div>
            <div class="feature-card">
                <h4>📊 Real-Time Stats</h4>
                <ul>
                    <li>Current view counts</li>
                    <li>Live engagement</li>
                    <li>Recent comments</li>
                    <li>Trending status</li>
                </ul>
            </div>
            <div class="feature-card">
                <h4>🤖 AI Insights</h4>
                <ul>
                    <li>Performance analysis</li>
                    <li>Optimization tips</li>
                    <li>Content suggestions</li>
                    <li>Competitor research</li>
                </ul>
            </div>
        </div>

        <div class="coming-soon">
            <strong>🔧 Planned next additions:</strong> Interactive charts, date ranges, export reports
        </div>

        <div style="margin-top: 30px; text-align: center;">
            <a href="/youtube/manage" class="btn">Manage Channels</a>
            <a href="/dashboard" class="btn">Dashboard</a>
        </div>
    </div>

    <script>
        class DynamicBackgroundManager {
            constructor() {
                this.updateBackground();
                setInterval(() => this.updateBackground(), 5 * 60 * 1000);
            }

            async updateBackground() {
                try {
                    const response = await fetch('/api/background/image');
                    if (response.ok) {
                        const blob = await response.blob();
                        const url = URL.createObjectURL(blob);
                        const overlay = document.createElement('div');
                        overlay.style.cssText = `position:fixed;top:0;left:0;width:100%;height:100%;background-image:url(${url});background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none`;
                        document.body.appendChild(overlay);
                        setTimeout(() => overlay.style.opacity = '0.3', 100);
                        setTimeout(() => {
                            const old = document.querySelectorAll('div[style*="background-image"]');
                            old.forEach((o, i) => { if (i < old.length - 1) o.remove(); });
                        }, 1100);
                    }
                } catch (e) { console.error('Background error:', e); }
            }
        }
        new DynamicBackgroundManager();
    </script>
</body>
</html>
    "###;
    Html(html.to_string())
}

// ============================================================================
// Help & Guide Page (with dark theme + dynamic background)
// ============================================================================

pub async fn help_guide_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>📖 Help - VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
            min-height: 100vh;
            color: #e8e8e8;
            padding: 20px;
        }

        .container {
            max-width: 1200px;
            margin: 0 auto;
            background: rgba(26, 26, 46, 0.95);
            backdrop-filter: blur(20px);
            border-radius: 20px;
            padding: 40px;
            box-shadow: 0 20px 60px rgba(0,0,0,0.5);
            border: 1px solid rgba(59, 130, 246, 0.3);
        }

        .nav-link { color: #3b82f6; text-decoration: none; font-weight: 600; display: inline-block; margin-bottom: 20px; }
        h1 { color: #fff; font-size: 2.5rem; margin-bottom: 10px; }
        .subtitle { color: #bdc3c7; margin-bottom: 30px; }

        .toc {
            background: rgba(255,255,255,0.05);
            padding: 20px;
            border-radius: 10px;
            margin-bottom: 30px;
            border: 1px solid rgba(59, 130, 246, 0.2);
        }

        .toc h3 { color: #3b82f6; margin-bottom: 15px; }
        .toc a { display: block; color: #3b82f6; text-decoration: none; padding: 5px 0; transition: padding-left 0.2s; }
        .toc a:hover { padding-left: 10px; }

        .section { margin-bottom: 40px; }
        .section h2 { color: #3b82f6; font-size: 1.8rem; margin-bottom: 15px; padding-bottom: 10px; border-bottom: 2px solid rgba(59, 130, 246, 0.3); }
        .section h3 { color: #fff; font-size: 1.3rem; margin: 20px 0 10px; }
        .section p { color: #bdc3c7; line-height: 1.8; margin-bottom: 15px; }
        .section ul, .section ol { margin-left: 30px; color: #bdc3c7; line-height: 1.8; }
        .section li { margin-bottom: 10px; }

        .example-box {
            background: rgba(59, 130, 246, 0.1);
            border-left: 4px solid #3b82f6;
            padding: 15px;
            border-radius: 5px;
            margin: 15px 0;
            font-family: monospace;
            color: #e8e8e8;
        }

        .warning-box {
            background: rgba(255, 193, 7, 0.1);
            border-left: 4px solid #ffc107;
            padding: 15px;
            border-radius: 5px;
            margin: 15px 0;
            color: #ffc107;
        }

        .success-box {
            background: rgba(40, 167, 69, 0.1);
            border-left: 4px solid #28a745;
            padding: 15px;
            border-radius: 5px;
            margin: 15px 0;
            color: #28a745;
        }

        .btn {
            display: inline-block;
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            padding: 15px 30px;
            border-radius: 25px;
            text-decoration: none;
            font-weight: 600;
            margin: 5px;
            transition: all 0.3s;
        }

        .btn:hover {
            transform: translateY(-2px);
            box-shadow: 0 10px 20px rgba(59, 130, 246, 0.3);
        }
    </style>
</head>
<body>
    <div class="container">
        <a href="/dashboard" class="nav-link">← Dashboard</a>
        <h1>📖 Help & User Guide</h1>
        <p class="subtitle">Complete guide to AI-powered video editing</p>

        <div class="toc">
            <h3>📑 Quick Links</h3>
            <a href="#start">Getting Started</a>
            <a href="#chat">AI Chat Commands</a>
            <a href="#edit">Video Editing</a>
            <a href="#workflows">Workflow Recipes</a>
            <a href="#video-tools">Video Tools Page</a>
            <a href="#youtube">YouTube Integration</a>
            <a href="#ai">AI Tools</a>
            <a href="#trouble">Troubleshooting</a>
        </div>

        <div class="section" id="start">
            <h2>1. Getting Started</h2>
            <h3>First Video Edit</h3>
            <ol>
                <li>Click <strong>Start New Chat</strong></li>
                <li>Upload video file (📎 button)</li>
                <li>Tell AI what you want</li>
                <li>Download result!</li>
            </ol>
            <div class="example-box">
"Make this black and white and add text Epic at 5 seconds"
            </div>
        </div>

        <div class="section" id="chat">
            <h2>2. AI Chat Commands</h2>
            <div class="example-box">
✂️ "Trim from 10 to 30 seconds"<br>
🎨 "Apply vintage filter"<br>
📝 "Add subtitles: Hello World"<br>
🎬 "Merge video1.mp4 and video2.mp4"<br>
📺 "Export for YouTube"
            </div>
        </div>

        <div class="section" id="edit">
            <h2>3. Professional Video Editing</h2>
            <p style="background:#eef2ff;border-radius:8px;padding:12px 16px;font-size:14px;color:#1e40af"><strong>No 60-second cap.</strong> Unlike other AI video generators, VideoSync handles videos of any length — short clips to feature-length productions.</p>
            <h3>Core Editing</h3>
            <ul>
                <li>Trim, Merge, Split, Resize, Crop, Rotate, Reverse, Loop</li>
                <li>2-pass video stabilization (vid.stab)</li>
                <li>Scene detection, black frame & silence detection</li>
            </ul>
            <h3>Visual Effects (100+ tools)</h3>
            <ul>
                <li>Color grading: LUT3D, curves, vibrance, HSV, hue/saturation</li>
                <li>Cinematic: film grain, vignette, vintage curves, telecine</li>
                <li>Keying: chroma key, luma key, color hold</li>
                <li>Spatial: motion blur, glow, bloom, sharpen, denoise</li>
                <li>Artistic: posterize (geq), solarize, CLAHE (histeq), banding fix</li>
            </ul>
            <h3>Audio Processing (80+ tools)</h3>
            <ul>
                <li>Loudness: EBU R128 / LUFS normalization, limiter (alimiter)</li>
                <li>Cleanup: RNN denoiser, de-esser (deesser), speech normalizer</li>
                <li>EQ: graphiceq, parametric eq, high/low shelf, bandpass</li>
                <li>Dynamics: compressor, expander, gate, sidechaining</li>
                <li>Visualize: CQT, spectrum analyzer, waveform video</li>
            </ul>
            <h3>Analysis (40+ tools)</h3>
            <ul>
                <li>Quality: VMAF, SSIM, PSNR</li>
                <li>Inspection: bitrate, metadata, scene change, freeze frames</li>
            </ul>
            <div class="example-box">
"Stabilize this shaky footage"<br>
"Apply film grain and a vignette for a cinematic look"<br>
"Normalize audio to -14 LUFS for YouTube"<br>
"Remove background noise and de-ess the sibilance"<br>
"Generate a CQT audio visualization"<br>
"Convert to MKV"
            </div>
        </div>

        <div class="section" id="workflows">
            <h2>4. Workflow Recipes</h2>
            <p>Named multi-step chains that apply several tools in sequence with a single command:</p>
            <ul>
                <li><strong>YouTube Ready Export</strong> — stabilize → normalize color → loudnorm −14 LUFS → yuv420p</li>
                <li><strong>Podcast Cleanup</strong> — denoise → de-ess sibilance → limit peaks → loudnorm −16 LUFS</li>
                <li><strong>Cinematic Grade</strong> — vintage curves → vibrance → vignette → film grain</li>
                <li><strong>Talking Head Cleanup</strong> — stabilize → denoise speech → de-ess → loudnorm −16 LUFS</li>
                <li><strong>GIF Creator</strong> — trim segment → scale → optimize palette</li>
            </ul>
            <div class="example-box">
"Run the YouTube ready export workflow on my video"<br>
"Apply podcast cleanup to this recording"<br>
"Give it a cinematic grade"<br>
"Create a GIF from seconds 10 to 15"
            </div>
        </div>

        <div class="section" id="video-tools">
            <h2>5. Video Tools Page</h2>
            <p>For direct tool access without the AI chat, visit <a href="/video-tools" style="color:#3b82f6;">/video-tools</a>. It provides four interactive panels:</p>
            <ul>
                <li><strong>Stabilize</strong> — shakiness/smoothing/zoom sliders, runs 2-pass vid.stab</li>
                <li><strong>Convert Format</strong> — dropdown of 11 output formats (mp4, mkv, webm, mov, wav, …)</li>
                <li><strong>Audio Visualizer</strong> — waveform / spectrum / CQT video output</li>
                <li><strong>Workflows</strong> — pick a recipe, enter file path, get a download link</li>
            </ul>
            <p>Enter the file path relative to <code>uploads/</code> (the filename shown after upload). A download link appears when processing completes.</p>
        </div>

        <div class="section" id="youtube">
            <h2>6. YouTube Integration</h2>
            <h3>Connect Channel</h3>
            <ol>
                <li>Go to <strong>Connect YouTube Channels</strong></li>
                <li>Sign in with Google</li>
                <li>Grant permissions</li>
            </ol>
            <h3>Features</h3>
            <ul>
                <li>Upload, Update metadata, Delete videos</li>
                <li>Custom thumbnails, Playlists</li>
                <li>Analytics, Comments, Captions</li>
            </ul>
            <div class="example-box">
AI YouTube Tools:<br>
"Optimize metadata for YouTube gaming"<br>
"What's trending in tech?"<br>
"Search for cooking channels"
            </div>
        </div>

        <div class="section" id="ai">
            <h2>7. AI Tools</h2>
            <ul>
                <li><strong>Stock Media:</strong> Free videos/photos from Pexels</li>
                <li><strong>TTS:</strong> 17+ natural voices (75ms latency)</li>
                <li><strong>Music:</strong> Studio-quality background tracks</li>
                <li><strong>Sound FX:</strong> Custom sound effects</li>
            </ul>
        </div>

        <div class="section" id="trouble">
            <h2>8. Troubleshooting</h2>
            <div class="warning-box">
                <strong>Stuck "Connecting":</strong> Hard refresh (Ctrl+Shift+R)
            </div>
            <div class="warning-box">
                <strong>YouTube Permissions:</strong> Reconnect at /youtube/connect
            </div>
            <div class="success-box">
                <strong>Pro Tip:</strong> Be specific with commands for best results!
            </div>
        </div>

        <div style="text-align: center; margin-top: 40px;">
            <a href="/chat" class="btn">🎬 Start Editing</a>
            <a href="/dashboard" class="btn">📊 Dashboard</a>
        </div>
    </div>

    <script>
        class DynamicBackgroundManager {
            constructor() {
                this.updateBackground();
                setInterval(() => this.updateBackground(), 5 * 60 * 1000);
            }
            async updateBackground() {
                try {
                    const r = await fetch('/api/background/image');
                    if (r.ok) {
                        const blob = await r.blob();
                        const url = URL.createObjectURL(blob);
                        const o = document.createElement('div');
                        o.style.cssText = `position:fixed;top:0;left:0;width:100%;height:100%;background-image:url(${url});background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none`;
                        document.body.appendChild(o);
                        setTimeout(() => o.style.opacity = '0.3', 100);
                        setTimeout(() => {
                            const old = document.querySelectorAll('div[style*="background-image"]');
                            old.forEach((e, i) => { if (i < old.length - 1) e.remove(); });
                        }, 1100);
                    }
                } catch (e) { console.error(e); }
            }
        }
        new DynamicBackgroundManager();
    </script>
</body>
</html>
    "###;
    Html(html.to_string())
}
// ============================================================================
// YouTube Clipping Management Page
// ============================================================================

pub async fn clipping_management_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>YouTube Clipping Manager - VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            min-height: 100vh;
            color: #e8e8e8;
        }
        .container { max-width: 1400px; margin: 0 auto; padding: 2rem; }
        .header {
            background: rgba(30, 30, 52, 0.8);
            backdrop-filter: blur(20px);
            border-bottom: 1px solid rgba(59, 130, 246, 0.3);
            padding: 1.5rem 2rem;
            margin-bottom: 2rem;
            border-radius: 15px;
        }
        .header h1 { font-size: 2rem; margin-bottom: 0.5rem; }
        .header p { color: #cbd5e1; font-size: 1rem; }
        .btn {
            padding: 0.75rem 1.5rem;
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            border: none;
            border-radius: 10px;
            font-weight: 600;
            cursor: pointer;
            transition: all 0.3s;
            text-decoration: none;
            display: inline-block;
        }
        .btn:hover {
            background: linear-gradient(135deg, #2563eb, #1e40af);
            transform: translateY(-2px);
            box-shadow: 0 4px 12px rgba(59, 130, 246, 0.4);
        }
        .btn-secondary { background: linear-gradient(135deg, #64748b, #475569); }
        .btn-secondary:hover { background: linear-gradient(135deg, #475569, #334155); }
        .btn-danger { background: linear-gradient(135deg, #ef4444, #dc2626); }
        .btn-danger:hover { background: linear-gradient(135deg, #dc2626, #b91c1c); }
        .btn-small { padding: 0.5rem 1rem; font-size: 0.9rem; }

        .tabs {
            display: flex;
            gap: 1rem;
            margin-bottom: 2rem;
            border-bottom: 2px solid rgba(59, 130, 246, 0.2);
        }
        .tab-button {
            padding: 1rem 2rem;
            background: none;
            border: none;
            color: #cbd5e1;
            cursor: pointer;
            border-bottom: 3px solid transparent;
            transition: all 0.3s;
            font-size: 1rem;
            font-weight: 500;
        }
        .tab-button.active {
            color: #3b82f6;
            border-bottom-color: #3b82f6;
        }
        .tab-button:hover { color: #60a5fa; }

        .tab-content { display: none; }
        .tab-content.active { display: block; }

        .card {
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 15px;
            padding: 1.5rem;
            margin-bottom: 1rem;
        }
        .grid {
            display: grid;
            grid-template-columns: repeat(auto-fill, minmax(350px, 1fr));
            gap: 1.5rem;
        }

        .channel-card {
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 15px;
            padding: 1.5rem;
            transition: all 0.3s;
        }
        .channel-card:hover {
            transform: translateY(-5px);
            box-shadow: 0 8px 20px rgba(59, 130, 246, 0.3);
            border-color: rgba(59, 130, 246, 0.5);
        }
        .channel-header {
            display: flex;
            gap: 1rem;
            margin-bottom: 1rem;
        }
        .channel-thumbnail {
            width: 60px;
            height: 60px;
            border-radius: 50%;
            object-fit: cover;
        }
        .channel-info h3 { font-size: 1.2rem; margin-bottom: 0.25rem; }
        .channel-info p { color: #94a3b8; font-size: 0.9rem; }

        .linkage-card {
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 15px;
            padding: 1.5rem;
        }
        .linkage-flow {
            display: flex;
            align-items: center;
            gap: 1rem;
            margin-bottom: 1rem;
        }
        .linkage-arrow { font-size: 1.5rem; color: #3b82f6; }

        .job-card {
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 15px;
            padding: 1.5rem;
            margin-bottom: 1rem;
        }
        .progress-bar {
            width: 100%;
            height: 8px;
            background: rgba(59, 130, 246, 0.2);
            border-radius: 10px;
            overflow: hidden;
            margin: 0.5rem 0;
        }
        .progress-fill {
            height: 100%;
            background: linear-gradient(90deg, #3b82f6, #60a5fa);
            transition: width 0.3s;
        }

        .status-badge {
            padding: 0.25rem 0.75rem;
            border-radius: 20px;
            font-size: 0.85rem;
            font-weight: 600;
        }
        .status-pending { background: rgba(251, 191, 36, 0.2); color: #fbbf24; }
        .status-running { background: rgba(59, 130, 246, 0.2); color: #3b82f6; }
        .status-completed { background: rgba(34, 197, 94, 0.2); color: #22c55e; }
        .status-failed { background: rgba(239, 68, 68, 0.2); color: #ef4444; }

        .modal {
            display: none;
            position: fixed;
            top: 0;
            left: 0;
            width: 100%;
            height: 100%;
            background: rgba(0, 0, 0, 0.7);
            z-index: 1000;
            align-items: center;
            justify-content: center;
        }
        .modal.active { display: flex; }
        .modal-content {
            background: rgba(30, 30, 52, 0.95);
            border: 1px solid rgba(59, 130, 246, 0.3);
            border-radius: 20px;
            padding: 2rem;
            max-width: 600px;
            width: 90%;
            max-height: 80vh;
            overflow-y: auto;
        }
        .modal-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 1.5rem;
        }
        .modal-close {
            background: none;
            border: none;
            color: #e8e8e8;
            font-size: 1.5rem;
            cursor: pointer;
        }

        .form-group {
            margin-bottom: 1.5rem;
        }
        .form-group label {
            display: block;
            margin-bottom: 0.5rem;
            color: #cbd5e1;
            font-weight: 500;
        }
        .form-group input,
        .form-group select {
            width: 100%;
            padding: 0.75rem;
            background: rgba(30, 30, 52, 0.8);
            border: 1px solid rgba(59, 130, 246, 0.3);
            border-radius: 10px;
            color: #e8e8e8;
            font-size: 1rem;
        }
        .form-group input:focus,
        .form-group select:focus {
            outline: none;
            border-color: #3b82f6;
        }

        .search-results {
            max-height: 400px;
            overflow-y: auto;
            margin-top: 1rem;
        }
        .search-result-item {
            display: flex;
            gap: 1rem;
            padding: 1rem;
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 10px;
            margin-bottom: 0.5rem;
            cursor: pointer;
            transition: all 0.3s;
        }
        .search-result-item:hover {
            background: rgba(30, 30, 52, 0.8);
            border-color: rgba(59, 130, 246, 0.5);
        }

        .loading {
            text-align: center;
            padding: 3rem;
            color: #94a3b8;
        }
        .empty-state {
            text-align: center;
            padding: 3rem;
            color: #94a3b8;
        }
        .empty-state-icon { font-size: 3rem; margin-bottom: 1rem; }

        .clip-card {
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 15px;
            padding: 1rem;
        }
        .clip-thumbnail {
            width: 100%;
            height: 200px;
            background: rgba(15, 20, 25, 0.8);
            border-radius: 10px;
            margin-bottom: 1rem;
            display: flex;
            align-items: center;
            justify-content: center;
            color: #64748b;
        }
        .viral-factors {
            display: flex;
            flex-wrap: wrap;
            gap: 0.5rem;
            margin: 0.5rem 0;
        }
        .viral-factor-badge {
            padding: 0.25rem 0.5rem;
            background: rgba(139, 92, 246, 0.2);
            color: #a78bfa;
            border-radius: 5px;
            font-size: 0.75rem;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <div style="display: flex; justify-content: space-between; align-items: center;">
                <div>
                    <h1>✂️ YouTube Clipping Manager</h1>
                    <p>Monitor channels, auto-clip viral moments, post to your channels</p>
                </div>
                <div>
                    <a href="/dashboard" class="btn btn-secondary">← Back to Dashboard</a>
                </div>
            </div>
        </div>

        <div class="tabs">
            <button class="tab-button active" onclick="switchTab('sources')">📺 Source Channels</button>
            <button class="tab-button" onclick="switchTab('linkages')">🔗 Channel Linkages</button>
            <button class="tab-button" onclick="switchTab('jobs')">⚙️ Active Jobs</button>
            <button class="tab-button" onclick="switchTab('clips')">🎬 Generated Clips</button>
            <button class="tab-button" onclick="switchTab('review')">🔍 Review Queue</button>
        </div>

        <!-- Tab 1: Source Channels -->
        <div id="sourcesTab" class="tab-content active">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem;">
                <h2>Source Channels to Monitor</h2>
                <button onclick="openAddChannelModal()" class="btn">+ Add Source Channel</button>
            </div>
            <div id="sourceChannelsContainer" class="loading">Loading source channels...</div>
        </div>

        <!-- Tab 2: Channel Linkages -->
        <div id="linkagesTab" class="tab-content">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem;">
                <h2>Channel Linkages</h2>
                <button onclick="openCreateLinkageModal()" class="btn">+ Create Linkage</button>
            </div>
            <div id="linkagesContainer" class="loading">Loading linkages...</div>
        </div>

        <!-- Tab 3: Clipping Jobs -->
        <div id="jobsTab" class="tab-content">
            <h2 style="margin-bottom: 1.5rem;">Active Clipping Jobs</h2>
            <div id="jobsContainer" class="loading">Loading jobs...</div>
        </div>

        <!-- Tab 4: Generated Clips -->
        <div id="clipsTab" class="tab-content">
            <h2 style="margin-bottom: 1.5rem;">Generated Clips Gallery</h2>
            <div id="clipsContainer" class="loading">Loading clips...</div>
        </div>

        <!-- Tab 5: Review Queue + Content Management -->
        <div id="reviewTab" class="tab-content">
            <h2 style="margin-bottom: 1.5rem;">Review Queue</h2>
            <p style="color: #94a3b8; margin-bottom: 1.5rem;">
                Clips awaiting human approval before upload. Enable per-linkage approval in the Linkages tab.
            </p>
            <div id="reviewContainer" class="loading">Loading pending clips...</div>

        </div>
    </div>

    <!-- Modal: Add Channel -->
    <div id="addChannelModal" class="modal">
        <div class="modal-content">
            <div class="modal-header">
                <h2>Add Source Channel</h2>
                <button class="modal-close" onclick="closeAddChannelModal()">×</button>
            </div>
            <div class="form-group">
                <label>Search for YouTube Channel</label>
                <input type="text" id="channelSearchInput" placeholder="e.g., MrBeast, PewDiePie..." oninput="searchChannels(this.value)">
            </div>
            <div id="channelSearchResults" class="search-results"></div>
        </div>
    </div>

    <!-- Modal: Create Linkage -->
    <div id="createLinkageModal" class="modal">
        <div class="modal-content">
            <div class="modal-header">
                <h2>Create Channel Linkage</h2>
                <button class="modal-close" onclick="closeCreateLinkageModal()">×</button>
            </div>
            <div class="form-group">
                <label>Source Channel (to clip from)</label>
                <select id="linkageSourceChannel"></select>
            </div>
            <div class="form-group">
                <label>Destination Channel (your channel)</label>
                <select id="linkageDestChannel"></select>
            </div>
            <div class="form-group">
                <label>Clips per Video (1-4)</label>
                <input type="number" id="linkageClipsPerVideo" min="1" max="4" value="2">
            </div>
            <div class="form-group">
                <label>Min Clip Duration (seconds)</label>
                <input type="number" id="linkageMinDuration" min="30" max="300" value="60">
            </div>
            <div class="form-group">
                <label>Max Clip Duration (seconds)</label>
                <input type="number" id="linkageMaxDuration" min="60" max="300" value="120">
            </div>
            <button onclick="createLinkage()" class="btn">Create Linkage</button>
        </div>
    </div>

    <script>
        const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
        if (!authToken) {
            window.location.href = '/login';
        }

        let searchTimeout = null;

        // Per-job WebSocket connections (job_id → WebSocket)
        const jobSockets = {};

        function openJobWebSocket(jobId) {
            if (jobSockets[jobId]) return;
            const proto = location.protocol === 'https:' ? 'wss' : 'ws';
            const ws = new WebSocket(proto + '://' + location.host + '/ws/clipping-jobs/' + jobId);
            jobSockets[jobId] = ws;

            ws.onmessage = function(event) {
                try { updateJobCard(jobId, JSON.parse(event.data)); } catch(e) {}
            };

            ws.onclose = function() {
                delete jobSockets[jobId];
                const card = document.getElementById('job-card-' + jobId);
                if (card && card.dataset.status === 'processing') {
                    setTimeout(function() { openJobWebSocket(jobId); }, 3000);
                }
            };
        }

        function updateJobCard(jobId, update) {
            const s = update.status;
            if (!s) return;
            const status = s.status;
            const step = s.current_step || '';
            const pct = s.progress_percent != null ? s.progress_percent : 0;
            const detail = s.current_action_detail || '';

            const stepEl = document.getElementById('job-step-' + jobId);
            if (stepEl) {
                stepEl.textContent = step;
            }

            const detailEl = document.getElementById('job-detail-' + jobId);
            if (detailEl) detailEl.textContent = detail;

            const barEl = document.getElementById('job-progress-bar-' + jobId);
            const pctEl = document.getElementById('job-progress-pct-' + jobId);
            if (barEl) barEl.style.width = pct + '%';
            if (pctEl) pctEl.textContent = Math.round(pct) + '%';

            const statusEl = document.getElementById('job-status-' + jobId);
            if (statusEl) {
                const classes = { running: 'status-running', completed: 'status-completed',
                                  failed: 'status-failed', queued: 'status-pending' };
                statusEl.className = 'status-badge ' + (classes[status] || 'status-running');
                statusEl.textContent = status;
            }

            const card = document.getElementById('job-card-' + jobId);
            if (card) card.dataset.status = status;

            if (status === 'completed' || status === 'failed') {
                if (jobSockets[jobId]) { jobSockets[jobId].close(); delete jobSockets[jobId]; }
                setTimeout(loadJobs, 2000);
            }
        }

        // Tab switching
        function switchTab(tabName) {
            // Close any open job WebSocket connections when leaving jobs tab
            if (tabName !== 'jobs') {
                Object.values(jobSockets).forEach(function(ws) { ws.close(); });
                Object.keys(jobSockets).forEach(function(k) { delete jobSockets[k]; });
            }

            document.querySelectorAll('.tab-button').forEach(btn => btn.classList.remove('active'));
            document.querySelectorAll('.tab-content').forEach(content => content.classList.remove('active'));

            event.target.classList.add('active');
            document.getElementById(tabName + 'Tab').classList.add('active');

            // Load data for the active tab
            if (tabName === 'sources') loadSourceChannels();
            else if (tabName === 'linkages') loadLinkages();
            else if (tabName === 'jobs') loadJobs();
            else if (tabName === 'clips') loadClips();
            else if (tabName === 'review') { loadPendingReview(); loadCMChannels(); }
        }

        // Load pending review clips
        async function loadPendingReview() {
            const container = document.getElementById('reviewContainer');
            container.className = 'loading';
            container.innerHTML = 'Loading pending clips...';

            try {
                const response = await fetch('/api/clipping/clips/pending-review', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                const clips = data.clips || [];

                if (clips.length === 0) {
                    container.className = 'empty-state';
                    container.innerHTML = '<div class="empty-state-icon">✅</div><h3>No clips pending review</h3><p>All clips have been reviewed or auto-published</p>';
                    return;
                }

                container.className = 'grid';
                container.innerHTML = clips.map(clip => `
                    <div class="clip-card" id="review-clip-${clip.id}">
                        <div class="clip-thumbnail">🎬 ${(clip.duration_seconds || 0).toFixed(1)}s</div>
                        <p style="color:#94a3b8; font-size:0.8rem; margin-bottom:0.5rem;">${clip.source_video_title || 'Unknown source'}</p>
                        ${(clip.qa_status || clip.qa_score || clip.qa_feedback) ? `
                            <div style="margin-bottom:0.75rem; padding:0.6rem 0.75rem; border-radius:10px; background:rgba(15,23,42,0.75); border:1px solid rgba(59,130,246,0.18);">
                                <div style="display:flex; justify-content:space-between; gap:0.75rem; align-items:center; margin-bottom:0.35rem;">
                                    <strong style="font-size:0.82rem; color:#cbd5e1;">QA ${clip.qa_status || 'not_reviewed'}</strong>
                                    <span style="font-size:0.8rem; color:#93c5fd;">${clip.qa_score != null ? `Score ${clip.qa_score}/10` : 'No score'}</span>
                                </div>
                                ${clip.qa_feedback ? `<div style="font-size:0.82rem; color:#94a3b8; line-height:1.35;">${clip.qa_feedback}</div>` : ''}
                                ${clip.qa_retry_hint ? `<div style="font-size:0.8rem; color:#fbbf24; margin-top:0.35rem;">Retry hint: ${clip.qa_retry_hint}</div>` : ''}
                            </div>
                        ` : ''}
                        <div class="form-group" style="margin-bottom:0.5rem;">
                            <label style="font-size:0.85rem;">Title</label>
                            <input type="text" id="review-title-${clip.id}" value="${(clip.proposed_title || clip.ai_title || '').replace(/"/g, '&quot;')}"
                                   style="width:100%; padding:0.5rem; background:rgba(30,30,52,0.8); border:1px solid rgba(59,130,246,0.3); border-radius:8px; color:#e8e8e8; font-size:0.9rem;">
                        </div>
                        <div class="form-group" style="margin-bottom:0.75rem;">
                            <label style="font-size:0.85rem;">Description</label>
                            <textarea id="review-desc-${clip.id}" rows="2"
                                      style="width:100%; padding:0.5rem; background:rgba(30,30,52,0.8); border:1px solid rgba(59,130,246,0.3); border-radius:8px; color:#e8e8e8; font-size:0.9rem; resize:vertical;">${clip.proposed_description || ''}</textarea>
                        </div>
                        <div style="display:flex; gap:0.5rem;">
                            <button onclick="saveAndApproveClip(${clip.id})" class="btn btn-small" style="background:linear-gradient(135deg,#22c55e,#16a34a);">Approve & Upload</button>
                            <button onclick="rejectReviewClip(${clip.id})" class="btn btn-danger btn-small">Reject</button>
                        </div>
                    </div>
                `).join('');
            } catch (error) {
                container.className = 'empty-state';
                container.innerHTML = '<p style="color:#ef4444;">Error: ' + error.message + '</p>';
            }
        }

        async function saveAndApproveClip(clipId) {
            const title = document.getElementById('review-title-' + clipId).value;
            const desc = document.getElementById('review-desc-' + clipId).value;

            // Save edits first
            if (title || desc) {
                await fetch('/api/clipping/clips/' + clipId + '/propose-edit', {
                    method: 'PUT',
                    headers: { 'Authorization': 'Bearer ' + authToken, 'Content-Type': 'application/json' },
                    body: JSON.stringify({ proposed_title: title, proposed_description: desc })
                });
            }

            // Approve (triggers upload)
            try {
                const r = await fetch('/api/clipping/clips/' + clipId + '/approve', {
                    method: 'PUT',
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await r.json();
                if (data.success) {
                    const card = document.getElementById('review-clip-' + clipId);
                    if (card) card.remove();
                    alert('Clip approved and uploading to YouTube!');
                } else {
                    alert('Approval failed. Check server logs.');
                }
            } catch (e) {
                alert('Error: ' + e.message);
            }
        }

        async function rejectReviewClip(clipId) {
            const reason = prompt('Reason for rejection (optional):');
            try {
                const r = await fetch('/api/clipping/clips/' + clipId + '/reject', {
                    method: 'PUT',
                    headers: { 'Authorization': 'Bearer ' + authToken, 'Content-Type': 'application/json' },
                    body: JSON.stringify({ reason })
                });
                if (r.ok) {
                    const card = document.getElementById('review-clip-' + clipId);
                    if (card) card.remove();
                } else {
                    alert('Rejection failed.');
                }
            } catch (e) {
                alert('Error: ' + e.message);
            }
        }

        // Load channels for content management agent dropdown
        async function loadCMChannels() {
            try {
                const r = await fetch('/api/youtube/channels', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await r.json();
                const channels = data.channels || [];
                document.getElementById('cmChannelSelect').innerHTML = channels.map(c =>
                    '<option value="' + c.id + '">' + c.channel_name + '</option>'
                ).join('') || '<option value="">No channels connected</option>';
            } catch (e) {}
        }

        // Load source channels
        async function loadSourceChannels() {
            const container = document.getElementById('sourceChannelsContainer');
            container.className = 'loading';
            container.innerHTML = 'Loading source channels...';

            try {
                const response = await fetch('/api/clipping/source-channels', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                const channels = data.channels || data || [];

                if (channels.length === 0) {
                    container.className = 'empty-state';
                    container.innerHTML = `
                        <div class="empty-state-icon">📺</div>
                        <h3>No Source Channels Yet</h3>
                        <p>Add channels to start monitoring for viral content</p>
                    `;
                    return;
                }

                container.className = 'grid';
                container.innerHTML = channels.map(channel => `
                    <div class="channel-card">
                        <div class="channel-header">
                            <img src="${channel.channel_thumbnail_url || '/placeholder.png'}"
                                 alt="${channel.channel_name}"
                                 class="channel-thumbnail">
                            <div class="channel-info">
                                <h3>${channel.channel_name}</h3>
                                <p>${channel.subscriber_count ? (channel.subscriber_count/1000000).toFixed(1) + 'M subscribers' : 'Unknown'}</p>
                            </div>
                        </div>
                        <p style="color: #94a3b8; font-size: 0.9rem; margin-bottom: 1rem;">
                            Polling every ${channel.polling_interval_minutes} minutes
                        </p>
                        <div style="display: flex; gap: 0.5rem;">
                            <button onclick="deleteSourceChannel(${channel.id})" class="btn btn-danger btn-small">Delete</button>
                            <button onclick="toggleChannelActive(${channel.id}, ${!channel.is_active})"
                                    class="btn btn-small ${channel.is_active ? 'btn-secondary' : ''}">
                                ${channel.is_active ? 'Deactivate' : 'Activate'}
                            </button>
                        </div>
                    </div>
                `).join('');
            } catch (error) {
                container.className = 'empty-state';
                container.innerHTML = `<p style="color: #ef4444;">Error loading channels: ${error.message}</p>`;
            }
        }

        // Load linkages
        async function loadLinkages() {
            const container = document.getElementById('linkagesContainer');
            container.className = 'loading';
            container.innerHTML = 'Loading linkages...';

            try {
                const response = await fetch('/api/clipping/linkages', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                const linkages = data.linkages || data || [];

                if (linkages.length === 0) {
                    container.className = 'empty-state';
                    container.innerHTML = `
                        <div class="empty-state-icon">🔗</div>
                        <h3>No Linkages Yet</h3>
                        <p>Create linkages to start auto-clipping</p>
                    `;
                    return;
                }

                container.innerHTML = linkages.map(linkage => `
                    <div class="linkage-card">
                        <div class="linkage-flow">
                            <div>
                                <strong>${linkage.source_channel_name}</strong>
                                <p style="color: #94a3b8; font-size: 0.85rem;">Source</p>
                            </div>
                            <div class="linkage-arrow">→</div>
                            <div>
                                <strong>${linkage.destination_channel_name}</strong>
                                <p style="color: #94a3b8; font-size: 0.85rem;">Destination</p>
                            </div>
                        </div>
                        <div style="background: rgba(59, 130, 246, 0.1); padding: 1rem; border-radius: 10px; margin-bottom: 1rem;">
                            <p><strong>Config:</strong> ${linkage.clips_per_video} clips/video, ${linkage.min_clip_duration_seconds}-${linkage.max_clip_duration_seconds}s duration</p>
                            <p><strong>Stats:</strong> ${linkage.total_clips_generated} generated, ${linkage.total_clips_posted} posted</p>
                        </div>
                        <div style="display: flex; gap: 0.5rem;">
                            <button onclick="deleteLinkage(${linkage.id})" class="btn btn-danger btn-small">Delete</button>
                            <button onclick="toggleLinkageActive(${linkage.id}, ${!linkage.is_active})"
                                    class="btn btn-small ${linkage.is_active ? 'btn-secondary' : ''}">
                                ${linkage.is_active ? 'Deactivate' : 'Activate'}
                            </button>
                        </div>
                    </div>
                `).join('');
            } catch (error) {
                container.className = 'empty-state';
                container.innerHTML = `<p style="color: #ef4444;">Error loading linkages: ${error.message}</p>`;
            }
        }

        // Load jobs
        async function loadJobs() {
            const container = document.getElementById('jobsContainer');

            try {
                const response = await fetch('/api/clipping/jobs', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                const jobs = data.jobs || data || [];

                if (jobs.length === 0) {
                    container.className = 'empty-state';
                    container.innerHTML = `
                        <div class="empty-state-icon">⚙️</div>
                        <h3>No Jobs Running</h3>
                        <p>Jobs will appear here when clipping starts</p>
                    `;
                    return;
                }

                container.innerHTML = jobs.map(job => {
                    const statusClass = job.status === 'completed' ? 'status-completed' :
                                       job.status === 'failed' ? 'status-failed' :
                                       job.status === 'pending' ? 'status-pending' : 'status-running';
                    const pct = job.progress_percent || 0;
                    const fallback = job.fallback_delivery;
                    const fallbackBlock = fallback ? `
                        <div style="margin-top:0.9rem; padding:0.85rem 1rem; border-radius:12px; background:rgba(15,23,42,0.72); border:1px solid rgba(96,165,250,0.2);">
                            <div style="display:flex; justify-content:space-between; gap:1rem; align-items:center; flex-wrap:wrap;">
                                <div>
                                    <div style="font-size:0.82rem; color:#93c5fd; margin-bottom:0.25rem;">Fallback delivery active</div>
                                    <div style="font-weight:600; color:#e2e8f0;">${fallback.title || 'Generated fallback delivery'}</div>
                                    <div style="font-size:0.8rem; color:#94a3b8; margin-top:0.2rem;">
                                        ${(job.fallback_strategy || 'generated_summary_delivery')} • ${fallback.status || 'pending'}
                                    </div>
                                    ${fallback.error_message ? `<div style="font-size:0.8rem; color:#fca5a5; margin-top:0.3rem;">${fallback.error_message}</div>` : ''}
                                </div>
                                <div style="display:flex; gap:0.5rem; flex-wrap:wrap;">
                                    ${fallback.output_r2_url ? `<a href="${fallback.output_r2_url}" target="_blank" class="btn btn-small" style="text-decoration:none;">Open Output</a>` : ''}
                                    ${fallback.delivery_page_url ? `<a href="${fallback.delivery_page_url}" target="_blank" class="btn btn-small btn-secondary" style="text-decoration:none;">Open Delivery</a>` : ''}
                                </div>
                            </div>
                        </div>
                    ` : '';

                    return `
                        <div class="job-card" id="job-card-${job.id}" data-status="${job.status}">
                            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem;">
                                <div>
                                    <h3>${job.source_video_title}</h3>
                                    <p id="job-step-${job.id}" style="color: #94a3b8; font-size: 0.9rem;">${job.current_step || 'Initializing'}</p>
                                </div>
                                <span id="job-status-${job.id}" class="status-badge ${statusClass}">${job.status}</span>
                            </div>
                            <div id="job-progress-wrap-${job.id}">
                                <div class="progress-bar">
                                    <div id="job-progress-bar-${job.id}" class="progress-fill" style="width: ${pct}%"></div>
                                </div>
                                <p id="job-progress-pct-${job.id}" style="text-align: right; color: #94a3b8; font-size: 0.85rem; margin-top: 0.25rem;">
                                    ${Math.round(pct)}%
                                </p>
                            </div>
                            ${job.error_message ? `<p style="color: #ef4444; margin-top: 0.5rem;">${job.error_message}</p>` : ''}
                            ${fallbackBlock}
                            <p id="job-detail-${job.id}" style="color: #64748b; font-size: 0.8rem; margin-top: 0.25rem;"></p>
                        </div>
                    `;
                }).join('');

                // Open WebSocket for each active job
                jobs.filter(j => j.status === 'processing' || j.status === 'pending').forEach(job => {
                    openJobWebSocket(job.id);
                });
            } catch (error) {
                container.className = 'empty-state';
                container.innerHTML = `<p style="color: #ef4444;">Error loading jobs: ${error.message}</p>`;
            }
        }

        // Load clips
        async function loadClips() {
            const container = document.getElementById('clipsContainer');
            container.className = 'loading';
            container.innerHTML = 'Loading clips...';

            try {
                const response = await fetch('/api/clipping/clips', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                const clips = data.clips || data || [];

                if (clips.length === 0) {
                    container.className = 'empty-state';
                    container.innerHTML = `
                        <div class="empty-state-icon">🎬</div>
                        <h3>No Clips Yet</h3>
                        <p>Generated clips will appear here</p>
                    `;
                    return;
                }

                container.className = 'grid';
                container.innerHTML = clips.map(clip => {
                    const statusClass = clip.upload_status === 'published' ? 'status-completed' :
                                       clip.upload_status === 'failed' ? 'status-failed' :
                                       clip.upload_status === 'uploading' ? 'status-running' : 'status-pending';

                    return `
                        <div class="clip-card">
                            <div class="clip-thumbnail">
                                🎬 ${clip.duration_seconds}s
                            </div>
                            <h3 style="margin-bottom: 0.5rem;">${clip.ai_title}</h3>
                            <span class="status-badge ${statusClass}">${clip.upload_status}</span>
                            ${clip.viral_factors && Array.isArray(clip.viral_factors) ? `
                                <div class="viral-factors">
                                    ${clip.viral_factors.map(factor =>
                                        `<span class="viral-factor-badge">${factor}</span>`
                                    ).join('')}
                                </div>
                            ` : ''}
                            ${clip.youtube_url ? `
                                <a href="${clip.youtube_url}" target="_blank" class="btn btn-small" style="margin-top: 0.5rem; width: 100%;">
                                    View on YouTube
                                </a>
                            ` : ''}
                        </div>
                    `;
                }).join('');
            } catch (error) {
                container.className = 'empty-state';
                container.innerHTML = `<p style="color: #ef4444;">Error loading clips: ${error.message}</p>`;
            }
        }

        // Channel search
        async function searchChannels(query) {
            if (!query || query.length < 2) {
                document.getElementById('channelSearchResults').innerHTML = '';
                return;
            }

            clearTimeout(searchTimeout);
            searchTimeout = setTimeout(async () => {
                try {
                    const response = await fetch(`/api/youtube/search-channels?q=${encodeURIComponent(query)}`, {
                        headers: { 'Authorization': 'Bearer ' + authToken }
                    });
                    const data = await response.json();

                    const resultsHtml = data.channels.map(channel => `
                        <div class="search-result-item" onclick="selectChannel('${channel.channel_id}', '${channel.channel_name.replace(/'/g, "\\'")}')">
                            <img src="${channel.thumbnail_url || '/placeholder.png'}"
                                 alt="${channel.channel_name}"
                                 style="width: 50px; height: 50px; border-radius: 50%;">
                            <div>
                                <strong>${channel.channel_name}</strong>
                                <p style="color: #94a3b8; font-size: 0.9rem;">${channel.description || 'No description'}</p>
                            </div>
                        </div>
                    `).join('');

                    document.getElementById('channelSearchResults').innerHTML = resultsHtml || '<p style="color: #94a3b8;">No channels found</p>';
                } catch (error) {
                    document.getElementById('channelSearchResults').innerHTML = `<p style="color: #ef4444;">Error: ${error.message}</p>`;
                }
            }, 500);
        }

        async function selectChannel(channelId, channelName) {
            try {
                const response = await fetch('/api/clipping/source-channels', {
                    method: 'POST',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({
                        channel_id: channelId,
                        polling_interval_minutes: 30
                    })
                });

                if (response.ok) {
                    closeAddChannelModal();
                    loadSourceChannels();
                } else {
                    const error = await response.json();
                    alert('Error adding channel: ' + error.message);
                }
            } catch (error) {
                alert('Error: ' + error.message);
            }
        }

        // Linkage creation
        async function createLinkage() {
            const sourceChannelId = document.getElementById('linkageSourceChannel').value;
            const destChannelId = document.getElementById('linkageDestChannel').value;
            const clipsPerVideo = document.getElementById('linkageClipsPerVideo').value;
            const minDuration = document.getElementById('linkageMinDuration').value;
            const maxDuration = document.getElementById('linkageMaxDuration').value;

            try {
                const response = await fetch('/api/clipping/linkages', {
                    method: 'POST',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({
                        source_channel_id: parseInt(sourceChannelId),
                        destination_channel_id: parseInt(destChannelId),
                        clips_per_video: parseInt(clipsPerVideo),
                        min_clip_duration_seconds: parseInt(minDuration),
                        max_clip_duration_seconds: parseInt(maxDuration)
                    })
                });

                if (response.ok) {
                    closeCreateLinkageModal();
                    loadLinkages();
                } else {
                    const error = await response.json();
                    alert('Error creating linkage: ' + error.message);
                }
            } catch (error) {
                alert('Error: ' + error.message);
            }
        }

        // Modal functions
        function openAddChannelModal() {
            document.getElementById('addChannelModal').classList.add('active');
        }

        function closeAddChannelModal() {
            document.getElementById('addChannelModal').classList.remove('active');
            document.getElementById('channelSearchInput').value = '';
            document.getElementById('channelSearchResults').innerHTML = '';
        }

        async function openCreateLinkageModal() {
            // Load source channels
            const sourcesResponse = await fetch('/api/clipping/source-channels', {
                headers: { 'Authorization': 'Bearer ' + authToken }
            });
            const sourcesData = await sourcesResponse.json();
            const sources = sourcesData.channels || sourcesData || [];

            // Load user's YouTube channels
            const channelsResponse = await fetch('/api/youtube/channels', {
                headers: { 'Authorization': 'Bearer ' + authToken }
            });
            const channelsData = await channelsResponse.json();
            const channels = channelsData.channels || channelsData || [];

            document.getElementById('linkageSourceChannel').innerHTML = sources.map(s =>
                `<option value="${s.id}">${s.channel_name}</option>`
            ).join('');

            document.getElementById('linkageDestChannel').innerHTML = channels.map(c =>
                `<option value="${c.id}">${c.channel_name}</option>`
            ).join('');

            document.getElementById('createLinkageModal').classList.add('active');
        }

        function closeCreateLinkageModal() {
            document.getElementById('createLinkageModal').classList.remove('active');
        }

        // Delete functions
        async function deleteSourceChannel(id) {
            if (!confirm('Are you sure you want to delete this source channel?')) return;

            try {
                await fetch(`/api/clipping/source-channels/${id}`, {
                    method: 'DELETE',
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                loadSourceChannels();
            } catch (error) {
                alert('Error: ' + error.message);
            }
        }

        async function deleteLinkage(id) {
            if (!confirm('Are you sure you want to delete this linkage?')) return;

            try {
                await fetch(`/api/clipping/linkages/${id}`, {
                    method: 'DELETE',
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                loadLinkages();
            } catch (error) {
                alert('Error: ' + error.message);
            }
        }

        // Toggle active status
        async function toggleChannelActive(id, isActive) {
            try {
                await fetch(`/api/clipping/source-channels/${id}`, {
                    method: 'PATCH',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({ is_active: isActive })
                });
                loadSourceChannels();
            } catch (error) {
                alert('Error: ' + error.message);
            }
        }

        async function toggleLinkageActive(id, isActive) {
            try {
                await fetch(`/api/clipping/linkages/${id}`, {
                    method: 'PATCH',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({ is_active: isActive })
                });
                loadLinkages();
            } catch (error) {
                alert('Error: ' + error.message);
            }
        }

        // Auto-refresh jobs every 5 seconds
        setInterval(() => {
            if (document.getElementById('jobsTab').classList.contains('active')) {
                loadJobs();
            }
        }, 5000);

        // Initial load
        loadSourceChannels();
    </script>
<script>
class DynamicBackgroundManager {
    constructor() { this.lastUpdate = Date.now(); this.interval = 5*60*1000; this.init(); }
    async init() { await this.updateBg(); setInterval(() => this.updateBg(), this.interval); }
    async updateBg() {
        try {
            const r = await fetch('/api/background/image');
            if (!r.ok) return;
            const ct = r.headers.get('content-type') || '';
            if (ct.includes('application/json')) {
                const d = await r.json();
                if (d.fallback && d.gradient) document.body.style.background = d.gradient;
                return;
            }
            const blob = await r.blob();
            const url = URL.createObjectURL(blob);
            const o = document.createElement('div');
            o.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;background-image:url('+url+');background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none';
            document.body.appendChild(o);
            setTimeout(() => o.style.opacity = '0.3', 100);
            setTimeout(() => {
                const old = document.querySelectorAll('div[style*="background-image"]');
                old.forEach((e,i) => { if (i < old.length - 1) e.remove(); });
            }, 1100);
        } catch(e) { console.error(e); }
    }
}
new DynamicBackgroundManager();
</script>
</body>
</html>
    "###;
    Html(html.to_string())
}

// ============================================================================
// Video Tools Page — SSR interactive tool UI (stabilize, convert, visualize, workflows)
// ============================================================================

pub async fn video_tools_page() -> Html<String> {
    let html = r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Video Tools — VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-attachment: fixed;
            min-height: 100vh;
            color: #e8e8e8;
            padding: 20px;
        }
        .topbar {
            max-width: 900px; margin: 0 auto 20px;
            display: flex; justify-content: space-between; align-items: center;
        }
        .topbar a { color: #3b82f6; text-decoration: none; font-weight: 600; }
        h1 { color: #fff; font-size: 2rem; }
        .subtitle { color: #9ca3af; margin-bottom: 24px; }
        .container { max-width: 900px; margin: 0 auto; }
        .tabs { display: flex; gap: 8px; margin-bottom: 24px; flex-wrap: wrap; }
        .tab-btn {
            padding: 8px 20px; border-radius: 20px; border: 1px solid rgba(59,130,246,0.4);
            background: rgba(59,130,246,0.1); color: #93c5fd; cursor: pointer;
            font-size: 0.875rem; transition: all 0.2s;
        }
        .tab-btn.active, .tab-btn:hover {
            background: rgba(59,130,246,0.3); border-color: #3b82f6; color: #fff;
        }
        .panel { display: none; }
        .panel.active { display: block; }
        .card {
            background: rgba(26,26,46,0.95); border-radius: 12px;
            border: 1px solid rgba(59,130,246,0.2); padding: 28px;
        }
        .card p { color: #9ca3af; margin-bottom: 20px; font-size: 0.9rem; line-height: 1.6; }
        .form-group { margin-bottom: 18px; }
        label { display: block; margin-bottom: 6px; font-size: 0.875rem; color: #d1d5db; }
        input[type=text], input[type=number], select {
            width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.05);
            border: 1px solid rgba(255,255,255,0.15); border-radius: 8px;
            color: #e8e8e8; font-size: 0.9rem; outline: none;
        }
        input:focus, select:focus { border-color: #3b82f6; }
        select option { background: #1e293b; }
        .slider-row { display: flex; align-items: center; gap: 12px; }
        .slider-row input[type=range] { flex: 1; }
        .slider-val { min-width: 30px; text-align: right; color: #3b82f6; font-weight: 600; }
        .two-col { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
        .btn {
            padding: 12px 28px; background: #3b82f6; color: #fff;
            border: none; border-radius: 8px; cursor: pointer; font-size: 0.9rem;
            font-weight: 600; transition: background 0.2s; width: 100%;
        }
        .btn:hover { background: #2563eb; }
        .btn:disabled { background: #374151; color: #6b7280; cursor: not-allowed; }
        .result { margin-top: 18px; padding: 14px; border-radius: 8px; font-size: 0.875rem; }
        .result.ok { background: rgba(34,197,94,0.1); border: 1px solid rgba(34,197,94,0.3); color: #86efac; }
        .result.err { background: rgba(239,68,68,0.1); border: 1px solid rgba(239,68,68,0.3); color: #fca5a5; }
        .download-link { display: inline-block; margin-top: 10px; color: #3b82f6; font-weight: 600; }
        .workflow-info { background: rgba(59,130,246,0.08); border: 1px solid rgba(59,130,246,0.2);
            border-radius: 6px; padding: 10px 14px; margin: 10px 0; font-size: 0.8rem; color: #93c5fd; }
        .gif-opts { margin-top: 14px; padding-top: 14px; border-top: 1px solid rgba(255,255,255,0.1); }
        code { font-size: 0.85em; background: rgba(255,255,255,0.08); padding: 2px 5px; border-radius: 3px; }
    </style>
</head>
<body>
    <div class="container">
        <div class="topbar">
            <a href="/dashboard">← Dashboard</a>
            <a href="/chat">💬 AI Chat</a>
        </div>
        <h1>🛠️ Video Tools</h1>
        <p class="subtitle">
            Direct access to professional video editing tools. Enter a file path relative to <code>uploads/</code>
            (e.g. <code>abc123_file.mp4</code>) — the name shown after upload in the chat.
        </p>

        <div class="tabs">
            <button class="tab-btn active" onclick="showTab('stabilize',this)">🎥 Stabilize</button>
            <button class="tab-btn" onclick="showTab('convert',this)">🔄 Convert Format</button>
            <button class="tab-btn" onclick="showTab('visualize',this)">🎵 Audio Visualizer</button>
            <button class="tab-btn" onclick="showTab('workflow',this)">⚡ Workflows</button>
        </div>

        <!-- ── Stabilize ───────────────────────────────────────────────── -->
        <div id="panel-stabilize" class="panel active">
            <div class="card">
                <p>Remove camera shake using 2-pass vid.stab analysis. Higher shakiness detects more motion; higher smoothing = more stable but more border crop.</p>
                <div class="form-group">
                    <label>Input file path</label>
                    <input type="text" id="stab-input" placeholder="uploads/my_video.mp4">
                </div>
                <div class="form-group">
                    <label>Shakiness: <span id="stab-shake-val">5</span></label>
                    <div class="slider-row">
                        <input type="range" id="stab-shake" min="1" max="10" value="5"
                            oninput="document.getElementById('stab-shake-val').textContent=this.value">
                        <span class="slider-val">1–10</span>
                    </div>
                </div>
                <div class="form-group">
                    <label>Smoothing: <span id="stab-smooth-val">10</span></label>
                    <div class="slider-row">
                        <input type="range" id="stab-smooth" min="1" max="50" value="10"
                            oninput="document.getElementById('stab-smooth-val').textContent=this.value">
                        <span class="slider-val">1–50</span>
                    </div>
                </div>
                <div class="form-group">
                    <label>Zoom: <span id="stab-zoom-val">0</span>%</label>
                    <div class="slider-row">
                        <input type="range" id="stab-zoom" min="0" max="20" value="0"
                            oninput="document.getElementById('stab-zoom-val').textContent=this.value">
                        <span class="slider-val">0–20%</span>
                    </div>
                </div>
                <button class="btn" id="stab-btn" onclick="runStabilize()">Stabilize Video</button>
                <div id="stab-result"></div>
            </div>
        </div>

        <!-- ── Convert Format ─────────────────────────────────────────── -->
        <div id="panel-convert" class="panel">
            <div class="card">
                <p>Convert to a different container format. Audio-only formats (mp3, wav, flac) extract the audio track.</p>
                <div class="form-group">
                    <label>Input file path</label>
                    <input type="text" id="conv-input" placeholder="uploads/my_video.mp4">
                </div>
                <div class="form-group">
                    <label>Target format</label>
                    <select id="conv-format">
                        <option value="mp4">MP4 (H.264)</option>
                        <option value="mkv">MKV (Matroska)</option>
                        <option value="webm">WebM (VP8/Vorbis)</option>
                        <option value="mov">MOV (QuickTime)</option>
                        <option value="avi">AVI</option>
                        <option value="ts">MPEG-TS</option>
                        <option value="mp3">MP3 (audio only)</option>
                        <option value="aac">AAC (audio only)</option>
                        <option value="flac">FLAC (lossless audio)</option>
                        <option value="wav">WAV (uncompressed)</option>
                        <option value="m4a">M4A (AAC in MP4)</option>
                    </select>
                </div>
                <button class="btn" id="conv-btn" onclick="runConvert()">Convert Format</button>
                <div id="conv-result"></div>
            </div>
        </div>

        <!-- ── Audio Visualizer ───────────────────────────────────────── -->
        <div id="panel-visualize" class="panel">
            <div class="card">
                <p>Generate an audio visualization video from any audio or video file. Result is an MP4 you can download.</p>
                <div class="form-group">
                    <label>Input file path</label>
                    <input type="text" id="viz-input" placeholder="uploads/my_audio.wav">
                </div>
                <div class="form-group">
                    <label>Visualization mode</label>
                    <select id="viz-mode">
                        <option value="waveform">Waveform (amplitude over time)</option>
                        <option value="spectrum">Spectrum (frequency intensity)</option>
                        <option value="cqt">CQT (musical frequency bands)</option>
                    </select>
                </div>
                <div class="two-col">
                    <div class="form-group">
                        <label>Width (px)</label>
                        <input type="number" id="viz-width" value="1280" min="320" max="3840" step="160">
                    </div>
                    <div class="form-group">
                        <label>Height (px)</label>
                        <input type="number" id="viz-height" value="400" min="100" max="1080" step="100">
                    </div>
                </div>
                <button class="btn" id="viz-btn" onclick="runVisualize()">Generate Visualization</button>
                <div id="viz-result"></div>
            </div>
        </div>

        <!-- ── Workflows ──────────────────────────────────────────────── -->
        <div id="panel-workflow" class="panel">
            <div class="card">
                <p>Named multi-step workflow chains that apply several editing tools in sequence.</p>
                <div class="form-group">
                    <label>Input file path</label>
                    <input type="text" id="wf-input" placeholder="uploads/my_video.mp4">
                </div>
                <div class="form-group">
                    <label>Workflow</label>
                    <select id="wf-select" onchange="updateWorkflowInfo()">
                        <option value="youtube_ready">YouTube Ready Export</option>
                        <option value="podcast_cleanup">Podcast Cleanup</option>
                        <option value="cinematic_grade">Cinematic Grade</option>
                        <option value="talking_head_cleanup">Talking Head Cleanup</option>
                        <option value="create_gif">Create GIF</option>
                    </select>
                </div>
                <div class="workflow-info" id="wf-info">
                    Stabilize → normalize color → loudnorm −14 LUFS → yuv420p
                </div>
                <div class="gif-opts" id="gif-opts" style="display:none;">
                    <div class="two-col">
                        <div class="form-group">
                            <label>Start (s)</label>
                            <input type="number" id="gif-start" value="0" min="0" step="1">
                        </div>
                        <div class="form-group">
                            <label>Duration (s)</label>
                            <input type="number" id="gif-dur" value="5" min="1" max="60" step="1">
                        </div>
                    </div>
                    <div class="two-col">
                        <div class="form-group">
                            <label>Width (px)</label>
                            <input type="number" id="gif-w" value="480" min="120" max="960" step="40">
                        </div>
                        <div class="form-group">
                            <label>FPS</label>
                            <input type="number" id="gif-fps" value="15" min="5" max="30" step="1">
                        </div>
                    </div>
                </div>
                <button class="btn" id="wf-btn" onclick="runWorkflow()">Run Workflow</button>
                <div id="wf-result"></div>
            </div>
        </div>
    </div>

    <script>
        const WORKFLOW_INFO = {
            youtube_ready: 'Stabilize → normalize color → loudnorm −14 LUFS → yuv420p',
            podcast_cleanup: 'Denoise → de-ess sibilance → limit peaks → loudnorm −16 LUFS',
            cinematic_grade: 'Vintage curves → vibrance → vignette → film grain',
            talking_head_cleanup: 'Stabilize → denoise speech → de-ess → loudnorm −16 LUFS',
            create_gif: 'Trim segment → scale → optimize palette (GIF output)',
        };

        function showTab(name, btn) {
            document.querySelectorAll('.panel').forEach(p => p.classList.remove('active'));
            document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
            document.getElementById('panel-' + name).classList.add('active');
            btn.classList.add('active');
        }

        function updateWorkflowInfo() {
            const wf = document.getElementById('wf-select').value;
            document.getElementById('wf-info').textContent = WORKFLOW_INFO[wf] || '';
            document.getElementById('gif-opts').style.display = wf === 'create_gif' ? 'block' : 'none';
        }

        function getAuthHeader() {
            const token = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
            return token ? { 'Authorization': 'Bearer ' + token } : {};
        }

        function showResult(id, result) {
            const el = document.getElementById(id);
            if (result.success) {
                el.className = 'result ok';
                el.innerHTML = escHtml(result.message)
                    + (result.download_url
                        ? '<br><a class="download-link" href="' + escHtml(result.download_url) + '" target="_blank">⬇ Download result</a>'
                        : '');
            } else {
                el.className = 'result err';
                el.textContent = result.message;
            }
        }

        function escHtml(s) {
            return String(s || '').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
        }

        async function postTool(endpoint, body, btnId, resultId) {
            const btn = document.getElementById(btnId);
            btn.disabled = true;
            btn.textContent = 'Processing…';
            document.getElementById(resultId).className = '';
            document.getElementById(resultId).textContent = '';
            try {
                const res = await fetch(endpoint, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json', ...getAuthHeader() },
                    body: JSON.stringify(body),
                });
                const data = await res.json();
                showResult(resultId, data);
            } catch (e) {
                showResult(resultId, { success: false, message: 'Request failed: ' + e.message });
            } finally {
                btn.disabled = false;
                btn.textContent = btn.getAttribute('data-label') || 'Run';
            }
        }

        // Set default labels
        document.getElementById('stab-btn').setAttribute('data-label', 'Stabilize Video');
        document.getElementById('conv-btn').setAttribute('data-label', 'Convert Format');
        document.getElementById('viz-btn').setAttribute('data-label', 'Generate Visualization');
        document.getElementById('wf-btn').setAttribute('data-label', 'Run Workflow');

        function runStabilize() {
            const input = document.getElementById('stab-input').value.trim();
            if (!input) return alert('Enter an input file path');
            postTool('/api/tools/stabilize', {
                input_file: input,
                shakiness: +document.getElementById('stab-shake').value,
                smoothing: +document.getElementById('stab-smooth').value,
                zoom: +document.getElementById('stab-zoom').value,
            }, 'stab-btn', 'stab-result');
        }

        function runConvert() {
            const input = document.getElementById('conv-input').value.trim();
            if (!input) return alert('Enter an input file path');
            postTool('/api/tools/convert', {
                input_file: input,
                format: document.getElementById('conv-format').value,
            }, 'conv-btn', 'conv-result');
        }

        function runVisualize() {
            const input = document.getElementById('viz-input').value.trim();
            if (!input) return alert('Enter an input file path');
            postTool('/api/tools/visualize-audio', {
                input_file: input,
                mode: document.getElementById('viz-mode').value,
                width: +document.getElementById('viz-width').value,
                height: +document.getElementById('viz-height').value,
            }, 'viz-btn', 'viz-result');
        }

        function runWorkflow() {
            const input = document.getElementById('wf-input').value.trim();
            if (!input) return alert('Enter an input file path');
            const wf = document.getElementById('wf-select').value;
            const body = { input_file: input, workflow: wf };
            if (wf === 'create_gif') {
                body.start_seconds = +document.getElementById('gif-start').value;
                body.duration_seconds = +document.getElementById('gif-dur').value;
                body.gif_width = +document.getElementById('gif-w').value;
                body.gif_fps = +document.getElementById('gif-fps').value;
            }
            postTool('/api/tools/workflow', body, 'wf-btn', 'wf-result');
        }
    </script>
<script>
class DynamicBackgroundManager {
    constructor() { this.lastUpdate = Date.now(); this.interval = 5*60*1000; this.init(); }
    async init() { await this.updateBg(); setInterval(() => this.updateBg(), this.interval); }
    async updateBg() {
        try {
            const r = await fetch('/api/background/image');
            if (!r.ok) return;
            const ct = r.headers.get('content-type') || '';
            if (ct.includes('application/json')) {
                const d = await r.json();
                if (d.fallback && d.gradient) document.body.style.background = d.gradient;
                return;
            }
            const blob = await r.blob();
            const url = URL.createObjectURL(blob);
            const o = document.createElement('div');
            o.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;background-image:url('+url+');background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none';
            document.body.appendChild(o);
            setTimeout(() => o.style.opacity = '0.3', 100);
            setTimeout(() => {
                const old = document.querySelectorAll('div[style*="background-image"]');
                old.forEach((e,i) => { if (i < old.length - 1) e.remove(); });
            }, 1100);
        } catch(e) { console.error(e); }
    }
}
new DynamicBackgroundManager();
</script>
</body>
</html>"###;
    Html(html.to_string())
}

// ============================================================================
// Privacy Policy Page
// ============================================================================

pub async fn privacy_policy_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Privacy Policy - VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-attachment: fixed;
            transition: background-image 1s;
            min-height: 100vh;
            color: #e8e8e8;
            padding: 20px;
            line-height: 1.6;
        }
        .container {
            max-width: 900px;
            margin: 0 auto;
            background: rgba(26, 26, 46, 0.95);
            backdrop-filter: blur(20px);
            border-radius: 20px;
            padding: 40px;
            box-shadow: 0 20px 60px rgba(0,0,0,0.5);
            border: 1px solid rgba(59, 130, 246, 0.3);
        }
        .nav-link { color: #3b82f6; text-decoration: none; font-weight: 600; display: inline-block; margin-bottom: 20px; }
        h1 { color: #fff; font-size: 2.5rem; margin-bottom: 10px; }
        h2 { color: #3b82f6; font-size: 1.8rem; margin: 30px 0 15px; padding-bottom: 10px; border-bottom: 2px solid rgba(59, 130, 246, 0.3); }
        h3 { color: #fff; font-size: 1.3rem; margin: 20px 0 10px; }
        p { color: #bdc3c7; margin-bottom: 15px; }
        ul, ol { margin-left: 30px; color: #bdc3c7; margin-bottom: 15px; }
        li { margin-bottom: 8px; }
        .date { color: #7f8c8d; font-size: 0.9rem; margin-bottom: 30px; }
        .section { margin-bottom: 30px; }
        strong { color: #fff; }
        a { color: #3b82f6; text-decoration: none; }
        a:hover { text-decoration: underline; }
        .highlight { background: rgba(59, 130, 246, 0.1); padding: 15px; border-left: 4px solid #3b82f6; border-radius: 5px; margin: 15px 0; }
    </style>
</head>
<body>
    <div class="container">
        <a href="/" class="nav-link">← Home</a>
        <h1>Privacy Policy</h1>
        <p class="date">Last Updated: December 23, 2025</p>

        <div class="section">
            <h2>Introduction</h2>
            <p>Agentic Video Editor ("VideoSync") operates an AI-powered video editing platform. This Privacy Policy explains how we collect, use, and protect your information.</p>
        </div>

        <div class="section">
            <h2>Information We Collect</h2>
            
            <h3>1. Account Information</h3>
            <ul>
                <li>Email address and username</li>
                <li>Password (encrypted)</li>
                <li>Profile information (if using Google sign-in)</li>
            </ul>

            <h3>2. YouTube Data (When Connected)</h3>
            <ul>
                <li>Channel name and statistics</li>
                <li>Video metadata (titles, descriptions, tags)</li>
                <li>Analytics data (views, engagement)</li>
                <li>OAuth tokens (encrypted)</li>
            </ul>

            <h3>3. Video Content</h3>
            <ul>
                <li>Videos you upload for editing</li>
                <li>Edited video outputs</li>
                <li>AI-generated content</li>
            </ul>
        </div>

        <div class="section">
            <h2>How We Use Your Information</h2>
            <ul>
                <li><strong>Video Editing</strong> - Process and edit your videos using AI</li>
                <li><strong>YouTube Integration</strong> - Upload and manage your YouTube content</li>
                <li><strong>AI Assistance</strong> - Generate metadata, voiceovers, and insights</li>
                <li><strong>Analytics</strong> - Show performance data from YouTube</li>
                <li><strong>Security</strong> - Prevent fraud and unauthorized access</li>
            </ul>
        </div>

        <div class="section">
            <h2>Third-Party Services</h2>
            <p>We integrate with:</p>
            <ul>
                <li><strong>Google</strong> - YouTube API, Gemini AI, OAuth</li>
                <li><strong>Anthropic</strong> - Claude AI for video editing</li>
                <li><strong>Eleven Labs</strong> - Voice and audio generation</li>
                <li><strong>Pexels</strong> - Stock media library</li>
            </ul>
        </div>

        <div class="section">
            <h2>YouTube Data Usage</h2>
            <div class="highlight">
                <p><strong>Important:</strong> We access YouTube data solely to provide features you request. You can revoke access anytime via <a href="https://myaccount.google.com/permissions" target="_blank">Google Account Permissions</a>.</p>
            </div>
            <p><strong>What we access:</strong> Channel info, video metadata, analytics, comments, playlists</p>
            <p><strong>What we DON'T do:</strong> We do NOT download other users' videos or share your data</p>
        </div>

        <div class="section">
            <h2>Data Security</h2>
            <ul>
                <li>✅ Encryption in transit (HTTPS/TLS)</li>
                <li>✅ Encrypted storage for OAuth tokens</li>
                <li>✅ Password hashing with bcrypt</li>
                <li>✅ JWT authentication</li>
                <li>✅ Rate limiting protection</li>
            </ul>
        </div>

        <div class="section">
            <h2>Your Rights</h2>
            <ul>
                <li><strong>Access</strong> - View all data we store about you</li>
                <li><strong>Delete</strong> - Request account and data deletion</li>
                <li><strong>Export</strong> - Download your videos and data</li>
                <li><strong>Disconnect</strong> - Revoke YouTube access anytime</li>
            </ul>
        </div>

        <div class="section">
            <h2>Data Retention</h2>
            <ul>
                <li>Active accounts: Data retained while account is active</li>
                <li>Temporary files: Deleted after 30 days</li>
                <li>Deleted accounts: All data purged within 30 days</li>
                <li>Analytics cache: 24 hours</li>
            </ul>
        </div>

        <div class="section">
            <h2>Contact Us</h2>
            <p>Questions about this Privacy Policy? Contact us at:</p>
            <p><strong>Email:</strong> support@yourapp.com</p>
            <p><strong>For data deletion:</strong> privacy@yourapp.com</p>
        </div>

        <div class="section">
            <h2>Compliance</h2>
            <p>This Privacy Policy complies with:</p>
            <ul>
                <li>General Data Protection Regulation (GDPR)</li>
                <li>California Consumer Privacy Act (CCPA)</li>
                <li><a href="https://developers.google.com/youtube/terms/api-services-terms-of-service" target="_blank">YouTube API Services Terms</a></li>
                <li><a href="https://developers.google.com/terms/api-services-user-data-policy" target="_blank">Google API Services User Data Policy</a></li>
            </ul>
        </div>

        <div style="text-align: center; margin-top: 40px; padding-top: 20px; border-top: 1px solid rgba(59, 130, 246, 0.3);">
            <a href="/" style="display: inline-block; background: linear-gradient(135deg, #3b82f6, #1d4ed8); color: white; padding: 12px 24px; border-radius: 25px; text-decoration: none; margin: 5px;">← Back to Home</a>
            <a href="/terms" style="display: inline-block; background: #6c757d; color: white; padding: 12px 24px; border-radius: 25px; text-decoration: none; margin: 5px;">View Terms of Service</a>
        </div>
    </div>

    <script>
        class DynamicBackgroundManager {
            constructor() { this.updateBackground(); setInterval(() => this.updateBackground(), 5*60*1000); }
            async updateBackground() {
                try {
                    const r = await fetch('/api/background/image');
                    if (r.ok) {
                        const blob = await r.blob();
                        const url = URL.createObjectURL(blob);
                        const o = document.createElement('div');
                        o.style.cssText = `position:fixed;top:0;left:0;width:100%;height:100%;background-image:url(${url});background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none`;
                        document.body.appendChild(o);
                        setTimeout(() => o.style.opacity = '0.3', 100);
                        setTimeout(() => {
                            const old = document.querySelectorAll('div[style*="background-image"]');
                            old.forEach((e, i) => { if (i < old.length - 1) e.remove(); });
                        }, 1100);
                    }
                } catch (e) { console.error(e); }
            }
        }
        new DynamicBackgroundManager();
    </script>
</body>
</html>
    "###;
    Html(html.to_string())
}

// ============================================================================
// Terms of Service Page
// ============================================================================

pub async fn terms_of_service_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Terms of Service - VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-attachment: fixed;
            transition: background-image 1s;
            min-height: 100vh;
            color: #e8e8e8;
            padding: 20px;
            line-height: 1.6;
        }
        .container {
            max-width: 900px;
            margin: 0 auto;
            background: rgba(26, 26, 46, 0.95);
            backdrop-filter: blur(20px);
            border-radius: 20px;
            padding: 40px;
            box-shadow: 0 20px 60px rgba(0,0,0,0.5);
            border: 1px solid rgba(59, 130, 246, 0.3);
        }
        .nav-link { color: #3b82f6; text-decoration: none; font-weight: 600; display: inline-block; margin-bottom: 20px; }
        h1 { color: #fff; font-size: 2.5rem; margin-bottom: 10px; }
        h2 { color: #3b82f6; font-size: 1.8rem; margin: 30px 0 15px; padding-bottom: 10px; border-bottom: 2px solid rgba(59, 130, 246, 0.3); }
        h3 { color: #fff; font-size: 1.3rem; margin: 20px 0 10px; }
        p { color: #bdc3c7; margin-bottom: 15px; }
        ul, ol { margin-left: 30px; color: #bdc3c7; margin-bottom: 15px; }
        li { margin-bottom: 8px; }
        .date { color: #7f8c8d; font-size: 0.9rem; margin-bottom: 30px; }
        .section { margin-bottom: 30px; }
        strong { color: #fff; }
        a { color: #3b82f6; text-decoration: none; }
        a:hover { text-decoration: underline; }
        .important { background: rgba(255, 193, 7, 0.1); padding: 15px; border-left: 4px solid #ffc107; border-radius: 5px; margin: 15px 0; color: #ffc107; }
        .highlight { background: rgba(59, 130, 246, 0.1); padding: 15px; border-left: 4px solid #3b82f6; border-radius: 5px; margin: 15px 0; }
    </style>
</head>
<body>
    <div class="container">
        <a href="/" class="nav-link">← Home</a>
        <h1>Terms of Service</h1>
        <p class="date">Last Updated: December 23, 2025</p>

        <div class="section">
            <h2>1. Acceptance of Terms</h2>
            <p>By using VideoSync, you agree to these Terms. If you don't agree, please don't use the Service.</p>
        </div>

        <div class="section">
            <h2>2. Service Description</h2>
            <p>VideoSync is an AI-powered video editing platform that enables you to:</p>
            <ul>
                <li>Edit videos using natural language commands</li>
                <li>Upload and manage YouTube videos</li>
                <li>Generate AI-powered content (voiceovers, music, thumbnails)</li>
                <li>Analyze YouTube channel performance</li>
                <li>Access stock media from Pexels</li>
            </ul>
        </div>

        <div class="section">
            <h2>3. User Accounts</h2>
            <h3>Account Creation</h3>
            <ul>
                <li>Provide accurate information</li>
                <li>Must be at least 13 years old</li>
                <li>Keep password secure</li>
                <li>One account per person</li>
            </ul>

            <h3>YouTube Connection</h3>
            <ul>
                <li>Optional - not required for video editing</li>
                <li>Connect multiple YouTube channels from different Google accounts</li>
                <li>Disconnect anytime</li>
                <li>We don't own your YouTube channels</li>
            </ul>
        </div>

        <div class="section">
            <h2>4. Acceptable Use</h2>
            
            <h3>You MAY:</h3>
            <ul>
                <li>✅ Edit your own videos</li>
                <li>✅ Upload to your YouTube channels</li>
                <li>✅ Use AI-generated content</li>
                <li>✅ Access stock media</li>
                <li>✅ Analyze your analytics</li>
            </ul>

            <h3>You MAY NOT:</h3>
            <ul>
                <li>❌ Upload copyrighted content without permission</li>
                <li>❌ Use for illegal activities</li>
                <li>❌ Harass or abuse others</li>
                <li>❌ Upload malware or viruses</li>
                <li>❌ Spam or send unsolicited messages</li>
                <li>❌ Download other users' YouTube videos</li>
            </ul>
        </div>

        <div class="section">
            <h2>5. Content Ownership</h2>
            <p><strong>Your Content:</strong> You retain full ownership of all videos you upload.</p>
            <p><strong>Our License:</strong> You grant us a limited license to process your content for editing purposes only.</p>
            <p><strong>AI-Generated Content:</strong> Provided "as-is" - you're responsible for ensuring it complies with laws.</p>
        </div>

        <div class="section">
            <h2>6. YouTube Integration</h2>
            <div class="highlight">
                <p>By using YouTube features, you agree to:</p>
                <ul style="margin-top: 10px;">
                    <li><a href="https://www.youtube.com/t/terms" target="_blank">YouTube Terms of Service</a></li>
                    <li><a href="https://developers.google.com/youtube/terms/api-services-terms-of-service" target="_blank">YouTube API Services Terms</a></li>
                    <li><a href="https://policies.google.com/privacy" target="_blank">Google Privacy Policy</a></li>
                </ul>
            </div>

            <h3>YouTube Permissions</h3>
            <p>We request:</p>
            <ul>
                <li><strong>youtube.upload</strong> - Upload videos to your channel</li>
                <li><strong>youtube.readonly</strong> - Read channel information</li>
                <li><strong>youtube.force-ssl</strong> - Modify videos, playlists, comments</li>
                <li><strong>yt-analytics.readonly</strong> - Access analytics</li>
            </ul>
            <p>You can revoke access anytime at <a href="https://myaccount.google.com/permissions" target="_blank">Google Account Permissions</a>.</p>
        </div>

        <div class="section">
            <h2>7. Service Limitations</h2>
            <ul>
                <li>Service provided "as-is" and "as available"</li>
                <li>File size limit: 500MB per video</li>
                <li>YouTube API has daily quota limits</li>
                <li>Scheduled maintenance may occur</li>
            </ul>
        </div>

        <div class="section">
            <h2>8. Prohibited Content</h2>
            <p>You may not upload content that:</p>
            <ul>
                <li>Violates copyright or intellectual property</li>
                <li>Contains hate speech or harassment</li>
                <li>Depicts violence or dangerous activities</li>
                <li>Contains sexually explicit material</li>
                <li>Violates YouTube Community Guidelines</li>
            </ul>
        </div>

        <div class="section">
            <h2>9. Disclaimers</h2>
            <div class="important">
                <p><strong>NO WARRANTIES:</strong> Service provided "as is" without guarantees.</p>
                <p><strong>LIMITED LIABILITY:</strong> Our liability is limited to $100 or amounts you paid in the past 12 months.</p>
            </div>
        </div>

        <div class="section">
            <h2>10. Account Termination</h2>
            <p><strong>By You:</strong> Delete account anytime from settings. All data deleted within 30 days.</p>
            <p><strong>By Us:</strong> We may suspend accounts that violate these Terms.</p>
        </div>

        <div class="section">
            <h2>11. Changes to Terms</h2>
            <p>We may update these Terms. Changes effective upon posting. Continued use = acceptance.</p>
        </div>

        <div class="section">
            <h2>12. Contact</h2>
            <p><strong>Email:</strong> support@yourapp.com</p>
            <p><strong>Legal inquiries:</strong> legal@yourapp.com</p>
        </div>

        <div style="text-align: center; margin-top: 40px; padding-top: 20px; border-top: 1px solid rgba(59, 130, 246, 0.3);">
            <a href="/" style="display: inline-block; background: linear-gradient(135deg, #3b82f6, #1d4ed8); color: white; padding: 12px 24px; border-radius: 25px; text-decoration: none; margin: 5px;">← Back to Home</a>
            <a href="/privacy" style="display: inline-block; background: #6c757d; color: white; padding: 12px 24px; border-radius: 25px; text-decoration: none; margin: 5px;">View Privacy Policy</a>
        </div>
    </div>

    <script>
        class DynamicBackgroundManager {
            constructor() { this.updateBackground(); setInterval(() => this.updateBackground(), 5*60*1000); }
            async updateBackground() {
                try {
                    const r = await fetch('/api/background/image');
                    if (r.ok) {
                        const blob = await r.blob();
                        const url = URL.createObjectURL(blob);
                        const o = document.createElement('div');
                        o.style.cssText = `position:fixed;top:0;left:0;width:100%;height:100%;background-image:url(${url});background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none`;
                        document.body.appendChild(o);
                        setTimeout(() => o.style.opacity = '0.3', 100);
                        setTimeout(() => {
                            const old = document.querySelectorAll('div[style*="background-image"]');
                            old.forEach((e, i) => { if (i < old.length - 1) e.remove(); });
                        }, 1100);
                    }
                } catch (e) { console.error(e); }
            }
        }
        new DynamicBackgroundManager();
    </script>
</body>
</html>
    "###;
    Html(html.to_string())
}

// ============================================================================
// Manual Clipping Dashboard
// ============================================================================

pub async fn manual_clipping_page() -> Html<String> {
    Html(MANUAL_CLIPPING_HTML.to_string())
}

const MANUAL_CLIPPING_HTML: &str = r#####"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Manual Clipping — VideoSync</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:'Inter',system-ui,sans-serif;background:#1a1a2e;color:#e0e0e0;min-height:100vh}
.header{background:#16213e;border-bottom:1px solid #0f3460;padding:14px 24px;display:flex;align-items:center;justify-content:space-between}
.header h1{font-size:1.1rem;color:#dbd8e3}
.back{color:#5c5470;text-decoration:none;font-size:0.9rem}.back:hover{color:#dbd8e3}
.container{max-width:900px;margin:0 auto;padding:24px}
.card{background:#16213e;border:1px solid #0f3460;border-radius:12px;padding:24px;margin-bottom:20px}
.card h2{font-size:1rem;color:#dbd8e3;margin-bottom:16px}
.url-row{display:flex;gap:12px;margin-bottom:12px}
.url-input{flex:1;padding:10px 14px;background:#0f3460;border:1px solid #1e3a5f;border-radius:8px;color:#e0e0e0;font-size:0.95rem}
.url-input:focus{outline:none;border-color:#5c5470}
.cfg-row{display:grid;grid-template-columns:repeat(3,1fr);gap:12px;margin-bottom:14px}
label{font-size:0.8rem;color:#9ca3af;display:block;margin-bottom:4px}
input[type=number]{width:100%;padding:8px 12px;background:#0f3460;border:1px solid #1e3a5f;border-radius:6px;color:#e0e0e0;font-size:0.9rem}
.btn{padding:10px 22px;border:none;border-radius:8px;cursor:pointer;font-size:0.9rem;font-weight:600}
.btn-primary{background:#5c5470;color:#fff}.btn-primary:hover{background:#7a6e8a}
.btn-sm{padding:5px 12px;font-size:0.8rem;border-radius:5px}
.btn-download{background:#065f46;color:#6ee7b7}.btn-download:hover{background:#047857}
.btn-cancel{background:#7f1d1d;color:#fca5a5}
.job-row{display:flex;align-items:flex-start;gap:12px;padding:14px;border-bottom:1px solid #0f3460}
.job-row:last-child{border-bottom:none}
.job-meta{flex:1}
.job-url{color:#dbd8e3;font-size:0.9rem;margin-bottom:4px;word-break:break-all}
.job-status{font-size:0.8rem;color:#9ca3af}
.status-dot{display:inline-block;width:8px;height:8px;border-radius:50%;margin-right:6px}
.status-pending,.status-analyzing,.status-downloading,.status-extracting,.status-uploading{background:#f59e0b}
.status-completed{background:#10b981}
.status-failed,.status-cancelled{background:#ef4444}
.clips-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(190px,1fr));gap:12px;margin-top:12px}
.clip-card{background:#0f3460;border:1px solid #1e3a5f;border-radius:8px;overflow:hidden}
.clip-thumb{width:100%;aspect-ratio:16/9;object-fit:cover}
.clip-thumb-ph{width:100%;aspect-ratio:16/9;background:#1a1a2e;display:flex;align-items:center;justify-content:center;color:#5c5470;font-size:1.5rem}
.clip-info{padding:8px 10px}
.clip-title{font-size:0.82rem;color:#dbd8e3;margin-bottom:4px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.clip-meta{font-size:0.75rem;color:#9ca3af;margin-bottom:8px}
.progress-bar{height:4px;background:#0f3460;border-radius:2px;overflow:hidden;margin-top:6px}
.progress-fill{height:100%;background:#5c5470;transition:width 0.3s}
.msg{padding:10px 16px;border-radius:8px;margin-bottom:12px;font-size:0.9rem}
.msg-success{background:#065f46;color:#6ee7b7}
.msg-error{background:#7f1d1d;color:#fca5a5}
.empty{text-align:center;padding:40px;color:#5c5470}
.platform-badge{font-size:0.72rem;padding:2px 7px;border-radius:4px;font-weight:600;margin-left:8px}
.yt-badge{background:#7f1d1d;color:#fca5a5}
.tw-badge{background:#2d1b69;color:#c4b5fd}
</style>
</head>
<body>
<div class="header">
  <div style="display:flex;align-items:center;gap:16px">
    <a href="/dashboard" id="back-link" class="back">Back</a>
    <h1>Manual Clipping</h1>
  </div>
  <span id="user-name" style="color:#9ca3af;font-size:0.85rem"></span>
</div>
<div class="container">
  <div id="msg"></div>
  <div class="card">
    <h2>Clip a Video</h2>
    <div class="url-row">
      <input class="url-input" id="video_url" placeholder="Paste YouTube or Twitch URL here" oninput="detectPlatform()">
      <button class="btn btn-primary" onclick="submitJob()">Clip It</button>
    </div>
    <div id="platform-hint" style="font-size:0.82rem;color:#9ca3af;margin-bottom:12px"></div>
    <div class="cfg-row">
      <div><label>Clips (1-5)</label><input type="number" id="clips_count" value="3" min="1" max="5"></div>
      <div><label>Min length (s)</label><input type="number" id="min_duration" value="30" min="10" max="300"></div>
      <div><label>Max length (s)</label><input type="number" id="max_duration" value="120" min="30" max="600"></div>
    </div>
    <p style="font-size:0.78rem;color:#5c5470">AI finds the best moments and generates download-ready clips. No YouTube channel required.</p>
  </div>
  <div class="card">
    <h2>Your Jobs</h2>
    <div id="jobs-area"><div class="empty">Loading...</div></div>
  </div>
</div>
<script>
const token = localStorage.getItem('authToken') || localStorage.getItem('admin_token') || localStorage.getItem('auth_token');
if (!token) window.location.href = '/login';
function parseJwt(t){try{return JSON.parse(atob(t.split('.')[1]));}catch(e){return{};}}
const claims = parseJwt(token);
if(claims.username) document.getElementById('user-name').textContent = claims.username;
if(!claims.is_clipper) document.getElementById('back-link').href='/dashboard';
function showMsg(text,ok=true){const el=document.getElementById('msg');el.innerHTML=`<div class="msg ${ok?'msg-success':'msg-error'}">${text}</div>`;setTimeout(()=>el.innerHTML='',4000);}
function detectPlatform(){const url=document.getElementById('video_url').value;const hint=document.getElementById('platform-hint');if(!url){hint.textContent='';return;}if(url.includes('twitch.tv'))hint.textContent='Twitch VOD detected';else if(url.includes('youtube.com')||url.includes('youtu.be'))hint.textContent='YouTube video detected';else hint.textContent='';}
async function submitJob(){const video_url=document.getElementById('video_url').value.trim();if(!video_url){showMsg('Please paste a URL',false);return;}const payload={video_url,clips_count:parseInt(document.getElementById('clips_count').value)||3,min_duration:parseInt(document.getElementById('min_duration').value)||30,max_duration:parseInt(document.getElementById('max_duration').value)||120};const res=await fetch('/api/manual-clipping/jobs',{method:'POST',headers:{'Authorization':'Bearer '+token,'Content-Type':'application/json'},body:JSON.stringify(payload)});const data=await res.json();if(data.success){showMsg('Job created! AI is analyzing...');document.getElementById('video_url').value='';loadJobs();}else showMsg(data.message||'Failed',false);}
let activeRefresh=null;
async function loadJobs(){const res=await fetch('/api/manual-clipping/jobs',{headers:{'Authorization':'Bearer '+token}});const data=await res.json();if(!data.success){document.getElementById('jobs-area').innerHTML='<div class="empty">Failed to load</div>';return;}const jobs=data.jobs||[];if(!jobs.length){document.getElementById('jobs-area').innerHTML='<div class="empty">No jobs yet. Paste a URL above to get started.</div>';return;}let html='';let hasActive=false;for(const j of jobs){const active=['pending','analyzing','downloading','extracting','uploading'].includes(j.status);if(active)hasActive=true;const platBadge=j.video_platform==='twitch'?'<span class="platform-badge tw-badge">TWITCH</span>':'<span class="platform-badge yt-badge">YOUTUBE</span>';html+=`<div class="job-row" id="job-${j.id}"><div class="job-meta"><div class="job-url">${j.video_title||j.video_url}${platBadge}</div><div class="job-status"><span class="status-dot status-${j.status}"></span>${j.status.toUpperCase()}${active?` - ${j.progress_percent}%`:''}${j.error_message?` <span style="color:#ef4444">${j.error_message}</span>`:''}</div>${active?`<div class="progress-bar"><div class="progress-fill" style="width:${j.progress_percent}%"></div></div>`:''} ${j.status==='completed'?`<div id="clips-${j.id}" style="margin-top:12px"></div>`:''}</div>${j.status==='completed'?`<button class="btn btn-sm btn-download" onclick="loadClips('${j.id}')">Load Clips</button>`:''}${active?`<button class="btn btn-sm btn-cancel" onclick="cancelJob('${j.id}')">Cancel</button>`:''}</div>`;}
document.getElementById('jobs-area').innerHTML=html;if(activeRefresh)clearTimeout(activeRefresh);if(hasActive)activeRefresh=setTimeout(loadJobs,5000);}
async function loadClips(jobId){const res=await fetch(`/api/manual-clipping/jobs/${jobId}`,{headers:{'Authorization':'Bearer '+token}});const data=await res.json();if(!data.success)return;const clips=data.clips||[];if(!clips.length)return;let html='<div class="clips-grid">';for(const c of clips){const dur=c.duration_seconds?Math.round(c.duration_seconds)+'s':'';const qaScore=c.qa_score!=null?`Score ${c.qa_score}/10`:'';const qaBlock=(c.qa_status||qaScore||c.qa_feedback||c.qa_retry_hint)?`<div style="margin-top:8px;padding:8px 10px;border-radius:10px;background:rgba(15,23,42,0.72);border:1px solid rgba(59,130,246,0.18)"><div style="font-size:0.78rem;color:#cbd5e1;margin-bottom:4px">QA ${c.qa_status||'not_reviewed'} ${qaScore?`• ${qaScore}`:''}</div>${c.qa_feedback?`<div style="font-size:0.78rem;color:#94a3b8;line-height:1.35">${c.qa_feedback}</div>`:''}${c.qa_retry_hint?`<div style="font-size:0.76rem;color:#fbbf24;margin-top:4px">Retry hint: ${c.qa_retry_hint}</div>`:''}</div>`:'';html+=`<div class="clip-card">${c.thumbnail_url?`<img class="clip-thumb" src="${c.thumbnail_url}" loading="lazy">`:'<div class="clip-thumb-ph">🎬</div>'}<div class="clip-info"><div class="clip-title">${c.title||'Clip '+c.clip_number}</div><div class="clip-meta">${dur}</div>${qaBlock}${c.download_url?`<a href="${c.download_url}" download class="btn btn-download" style="display:block;text-align:center;text-decoration:none;font-size:0.82rem;padding:7px;margin-top:8px">Download</a>`:'<span style="color:#9ca3af;font-size:0.8rem;display:block;margin-top:8px">Link expired</span>'}</div></div>`;}html+='</div>';const el=document.getElementById(`clips-${jobId}`);if(el)el.innerHTML=html;}
async function cancelJob(id){const res=await fetch(`/api/manual-clipping/jobs/${id}`,{method:'DELETE',headers:{'Authorization':'Bearer '+token}});const data=await res.json();if(data.success)loadJobs();else showMsg('Could not cancel',false);}
loadJobs();
</script>
<script>
class DynamicBackgroundManager {
    constructor() { this.lastUpdate = Date.now(); this.interval = 5*60*1000; this.init(); }
    async init() { await this.updateBg(); setInterval(() => this.updateBg(), this.interval); }
    async updateBg() {
        try {
            const r = await fetch('/api/background/image');
            if (!r.ok) return;
            const ct = r.headers.get('content-type') || '';
            if (ct.includes('application/json')) {
                const d = await r.json();
                if (d.fallback && d.gradient) document.body.style.background = d.gradient;
                return;
            }
            const blob = await r.blob();
            const url = URL.createObjectURL(blob);
            const o = document.createElement('div');
            o.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;background-image:url('+url+');background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none';
            document.body.appendChild(o);
            setTimeout(() => o.style.opacity = '0.3', 100);
            setTimeout(() => {
                const old = document.querySelectorAll('div[style*="background-image"]');
                old.forEach((e,i) => { if (i < old.length - 1) e.remove(); });
            }, 1100);
        } catch(e) { console.error(e); }
    }
}
new DynamicBackgroundManager();
</script>
</body>
</html>"#####;

// ============================================================================
// Clipper Invite Signup Page
// ============================================================================

pub async fn clipper_signup_page() -> Html<String> {
    Html(CLIPPER_SIGNUP_HTML.to_string())
}

const CLIPPER_SIGNUP_HTML: &str = r#####"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Join as Clipper — VideoSync</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:'Inter',system-ui,sans-serif;background:#1a1a2e;color:#e0e0e0;min-height:100vh;display:flex;align-items:center;justify-content:center}
.card{background:#16213e;border:1px solid #0f3460;border-radius:16px;padding:40px;width:100%;max-width:420px;margin:20px}
.logo{font-size:1.4rem;font-weight:700;color:#dbd8e3;margin-bottom:6px}
.subtitle{color:#9ca3af;font-size:0.9rem;margin-bottom:24px}
label{font-size:0.82rem;color:#9ca3af;display:block;margin-bottom:4px}
input{width:100%;padding:10px 14px;background:#0f3460;border:1px solid #1e3a5f;border-radius:8px;color:#e0e0e0;font-size:0.9rem;margin-bottom:14px}
input:focus{outline:none;border-color:#5c5470}
.btn{width:100%;padding:12px;background:#5c5470;color:#fff;border:none;border-radius:8px;cursor:pointer;font-size:0.95rem;font-weight:600;margin-top:4px}
.btn:hover{background:#7a6e8a}
.msg{padding:10px 14px;border-radius:8px;margin-bottom:14px;font-size:0.85rem}
.msg-success{background:#065f46;color:#6ee7b7}
.msg-error{background:#7f1d1d;color:#fca5a5}
.note{font-size:0.78rem;color:#5c5470;margin-top:12px;text-align:center}
</style>
</head>
<body>
<div class="card">
  <div class="logo">VideoSync</div>
  <div class="subtitle">Create your Clipper account</div>
  <div id="msg"></div>
  <form onsubmit="register(event)">
    <label>Invite Token</label>
    <input id="token" placeholder="Your invite token" required>
    <label>Email</label>
    <input type="email" id="email" placeholder="you@example.com" required>
    <label>Username</label>
    <input id="username" placeholder="clipper_name" required>
    <label>Password</label>
    <input type="password" id="password" placeholder="Min 6 characters" required>
    <label>Confirm Password</label>
    <input type="password" id="confirm_password" required>
    <button type="submit" class="btn">Create Account</button>
  </form>
  <p class="note">Already have an account? <a href="/login" style="color:#dbd8e3">Sign in</a></p>
</div>
<script>
const params=new URLSearchParams(window.location.search);
if(params.get('token'))document.getElementById('token').value=params.get('token');
function showMsg(text,ok=true){document.getElementById('msg').innerHTML=`<div class="msg ${ok?'msg-success':'msg-error'}">${text}</div>`;}
async function register(e){e.preventDefault();const payload={token:document.getElementById('token').value.trim(),email:document.getElementById('email').value.trim(),username:document.getElementById('username').value.trim(),password:document.getElementById('password').value,confirm_password:document.getElementById('confirm_password').value};const res=await fetch('/api/auth/register/clipper',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(payload)});const data=await res.json();if(data.success){localStorage.setItem('authToken',data.token);localStorage.setItem('auth_token',data.token);localStorage.setItem('user',JSON.stringify(data.user));localStorage.setItem('auth_user',JSON.stringify(data.user));showMsg('Account created! Redirecting...');setTimeout(()=>window.location.href='/manual-clipping',1200);}else showMsg(data.message||'Registration failed',false);}
</script>
<script>
class DynamicBackgroundManager {
    constructor() { this.lastUpdate = Date.now(); this.interval = 5*60*1000; this.init(); }
    async init() { await this.updateBg(); setInterval(() => this.updateBg(), this.interval); }
    async updateBg() {
        try {
            const r = await fetch('/api/background/image');
            if (!r.ok) return;
            const ct = r.headers.get('content-type') || '';
            if (ct.includes('application/json')) {
                const d = await r.json();
                if (d.fallback && d.gradient) document.body.style.background = d.gradient;
                return;
            }
            const blob = await r.blob();
            const url = URL.createObjectURL(blob);
            const o = document.createElement('div');
            o.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;background-image:url('+url+');background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none';
            document.body.appendChild(o);
            setTimeout(() => o.style.opacity = '0.3', 100);
            setTimeout(() => {
                const old = document.querySelectorAll('div[style*="background-image"]');
                old.forEach((e,i) => { if (i < old.length - 1) e.remove(); });
            }, 1100);
        } catch(e) { console.error(e); }
    }
}
new DynamicBackgroundManager();
</script>
</body>
</html>"#####;
