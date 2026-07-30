use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Crypto SaaS targets (legacy) ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct PortfolioTarget {
    pub slug: &'static str,
    pub company: &'static str,
    pub url: &'static str,
    pub market: &'static str,
    pub angle: &'static str,
    pub visual_direction: &'static str,
}

pub fn crypto_saas_targets() -> &'static [PortfolioTarget] {
    &[
        PortfolioTarget { slug: "privy", company: "Privy", url: "https://www.privy.io/", market: "wallet infrastructure", angle: "embedded wallets, programmable keys, and secure user onboarding", visual_direction: "premium wallet-stack command center with secure key shards, passkey login, and flowing transaction rails" },
        PortfolioTarget { slug: "dynamic", company: "Dynamic", url: "https://www.dynamic.xyz/", market: "wallet infrastructure for fintech and stablecoin apps", angle: "sub-second signing, embedded wallets, and revenue-ready crypto features", visual_direction: "enterprise fintech dashboard with live wallet activity, stablecoin motion trails, and fast-signing status pulses" },
        PortfolioTarget { slug: "crossmint", company: "Crossmint", url: "https://www.crossmint.com/", market: "stablecoin and wallet infrastructure", angle: "wallets, onramps, stablecoin orchestration, checkout, and tokenization APIs", visual_direction: "clean orchestration map with wallets, onramps, stablecoin transfers, and agent treasury nodes" },
        PortfolioTarget { slug: "thirdweb", company: "thirdweb", url: "https://thirdweb.com/", market: "web3 developer and agent infrastructure", angle: "AI agents, native internet payments, wallets, contracts, and blockchain data access", visual_direction: "developer launch bay with agent wallets, contract APIs, payment streams, and blockchain data panels" },
        PortfolioTarget { slug: "coinbase-business", company: "Coinbase Business", url: "https://www.coinbase.com/commerce", market: "crypto business payments", angle: "global payments, business accounts, API payouts, and USDC treasury workflows", visual_direction: "modern business finance workspace with global payment arcs, USDC treasury cards, and checkout confirmations" },
    ]
}

pub fn client_ref_for(target: &PortfolioTarget) -> String {
    format!("portfolio:crypto-saas:{}", target.slug)
}

pub fn build_crypto_saas_prompt(target: &PortfolioTarget) -> String {
    format!(
        "Create a speculative 15-second portfolio sample for a Web3 SaaS company inspired by {company} ({url}). \
         The video should sell {market}: {angle}. \
         Build a polished website-to-video explainer with cinematic camera moves, crisp UI panels, subtle 3D depth, \
         professional typography, crypto-native settlement cues, and a clear CTA. \
         Do not copy trademarks or exact brand artwork; create an original demo that shows VideoSync can turn a live website into a premium sales video.",
        company = target.company, url = target.url, market = target.market, angle = target.angle,
    )
}

pub fn build_crypto_saas_extra(target: &PortfolioTarget, reference_image_url: Option<&str>) -> Value {
    let narration_text = format!(
        "{company} powers {market}. This speculative VideoSync sample shows {angle} in a polished website-to-video explainer built for outbound sales.",
        company = target.company, market = target.market, angle = target.angle,
    );
    json!({
        "portfolio_category": "crypto_saas",
        "company": target.company,
        "source_url": target.url,
        "animation_style": "website_to_video",
        "reference_image_url": reference_image_url.unwrap_or_default(),
        "visual_direction": target.visual_direction,
        "include_narration": true,
        "narration_text": narration_text,
        "sales_positioning": "Use as a speculative outbound portfolio sample for crypto SaaS/startup prospects.",
        "compliance_note": "Speculative demo only; not commissioned by or affiliated with the referenced company."
    })
}

// ── 12 DFY Service targets ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DfyServiceDef {
    pub slug: &'static str,
    pub name: &'static str,
    pub price_mo: u32,
    pub brief: &'static str,
    pub style: &'static str,
    pub duration_seconds: f64,
    pub source_url: &'static str,
}

pub fn dfy_services() -> &'static [DfyServiceDef] {
    &[
        DfyServiceDef {
            slug: "clipping",
            name: "Short-Form Clip Distribution",
            price_mo: 297,
            brief: "Turn a 30-minute tech podcast episode into 5 high-retention short-form clips with captions, branded lower thirds, and viral hooks for TikTok and YouTube Shorts.",
            style: "high-retention clip pack, captions, branded lower thirds, viral hooks",
            duration_seconds: 15.0,
            source_url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        },
        DfyServiceDef {
            slug: "kick_auto_clipper",
            name: "Kick Auto-Clipper",
            price_mo: 297,
            brief: "Automatically clip a 2-hour Kick streaming VOD into 10 viral highlights with Kick-compliant branding (logo, captions, zooms, outro card), optimized for TikTok/Shorts vertical format.",
            style: "Kick clip style, vertical 9:16, karaoke captions, logo overlay, zoom on reactions",
            duration_seconds: 45.0,
            source_url: "https://kick.com/example",
        },
        DfyServiceDef {
            slug: "landing_page",
            name: "Business Landing Page Video",
            price_mo: 149,
            brief: "Create a 30-second animated hero video for a boutique consulting firm's homepage. Showcase their expertise with clean motion graphics, subtle 3D depth, professional typography, and a clear value proposition.",
            style: "premium business explainer, cinematic, clean motion graphics",
            duration_seconds: 30.0,
            source_url: "https://example.com",
        },
        DfyServiceDef {
            slug: "education",
            name: "Educational Explainer Lessons",
            price_mo: 199,
            brief: "Create a 60-second animated explainer about how neural networks learn. Use Manim-powered animations to visualize gradient descent, backpropagation, and weight updates. Clear narration with mathematical diagrams.",
            style: "clear narrated educational explainer, technical diagrams, Manim animations",
            duration_seconds: 60.0,
            source_url: "",
        },
        DfyServiceDef {
            slug: "manim_explainer",
            name: "Manim-Animated Explainer Campaign",
            price_mo: 149,
            brief: "Create a 45-second Manim-powered animated explainer about how blockchain consensus works. Clean motion graphics, narrated walkthrough, no 3D required.",
            style: "Manim-powered animated explainer, clean motion graphics, narrated",
            duration_seconds: 45.0,
            source_url: "",
        },
        DfyServiceDef {
            slug: "whiteboard_animation",
            name: "Whiteboard Animation Campaign",
            price_mo: 149,
            brief: "Create a 30-second whiteboard animation explaining the concept of compound interest. Hand-drawn sketch style, marker-on-board, educational tone with clear narration.",
            style: "hand-drawn whiteboard sketch style, marker-on-board, educational, narrated",
            duration_seconds: 30.0,
            source_url: "",
        },
        DfyServiceDef {
            slug: "kinetic_typography",
            name: "Kinetic Typography Campaign",
            price_mo: 149,
            brief: "Create a 20-second kinetic typography video from a famous motivational speech quote. Word-by-word reveal, dynamic text animation, narrated with energy.",
            style: "dynamic text animation, word-by-word reveal, kinetic type, narrated",
            duration_seconds: 20.0,
            source_url: "",
        },
        DfyServiceDef {
            slug: "animated_infographic",
            name: "Animated Infographic Campaign",
            price_mo: 149,
            brief: "Create a 30-second animated infographic showing global renewable energy adoption trends (2015-2025). Data-driven charts, counters, statistics, with clear narrated insights.",
            style: "data-driven animated infographic, charts, counters, statistics, narrated",
            duration_seconds: 30.0,
            source_url: "",
        },
        DfyServiceDef {
            slug: "algorithm_viz",
            name: "Algorithm Visualization Campaign",
            price_mo: 149,
            brief: "Create a 45-second visualization of the QuickSort algorithm. Show the divide-and-conquer process with animated array partitions, pivot selections, and recursive calls. Narrated technical explainer.",
            style: "algorithm visualization, code execution flow, data structures, narrated technical explainer",
            duration_seconds: 45.0,
            source_url: "",
        },
        DfyServiceDef {
            slug: "investor_pitch",
            name: "Investor Pitch Deck Campaign",
            price_mo: 149,
            brief: "Create a 60-second investor pitch video for an AI-powered SaaS startup. Motion graphics showcasing the problem, solution, market size, traction, and team. Clean brand presentation with professional narration.",
            style: "professional investor pitch video, motion graphics, narrated, clean brand presentation",
            duration_seconds: 60.0,
            source_url: "",
        },
        DfyServiceDef {
            slug: "year_in_review",
            name: "Year-in-Review Campaign",
            price_mo: 149,
            brief: "Create a 30-second year-in-review recap for a YouTube creator. Show their growth stats: subscribers (+340K), views (12.4M), top 3 videos, total watch time. Wrapped-style presentation with data-driven highlights.",
            style: "personalized year-in-review recap, data-driven highlights, wrapped-style, narrated",
            duration_seconds: 30.0,
            source_url: "",
        },
        DfyServiceDef {
            slug: "isometric_explainer",
            name: "Isometric Explainer Campaign",
            price_mo: 149,
            brief: "Create a 30-second isometric 3D explainer showing how a cloud CI/CD pipeline works. Code commit -> test -> build -> deploy stages visualized in angled perspective view with clean motion graphics.",
            style: "isometric 3D explaingit ner, angled perspective view, clean motion graphics, narrated",
            duration_seconds: 30.0,
            source_url: "",
        },
    ]
}

/// Build an AgenticServicePipeline ServiceInput for a DFY service.
pub fn service_input_for(def: &DfyServiceDef) -> crate::services::agentic_service_pipeline::ServiceInput {
    crate::services::agentic_service_pipeline::ServiceInput {
        service_type: crate::services::agentic_service_pipeline::ServiceType::from_normalized(def.slug),
        brief: def.brief.to_string(),
        style: def.style.to_string(),
        duration_seconds: def.duration_seconds,
        source_url: if def.source_url.is_empty() { None } else { Some(def.source_url.to_string()) },
        reference_image_url: None,
        extra_args: json!({
            "portfolio_category": "dfy_service_demo",
            "service_slug": def.slug,
            "service_name": def.name,
            "price_mo": def.price_mo,
            "sample_visibility": "shared_seed",
            "is_shared_seed": true,
        }),
    }
}

/// Generate a delivery-style client_ref for a DFY portfolio sample.
pub fn dfy_client_ref(slug: &str) -> String {
    format!("portfolio:dfy:{}", slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crypto_saas_targets_are_unique_and_complete() {
        let targets = crypto_saas_targets();
        assert_eq!(targets.len(), 5);
        let mut slugs = targets.iter().map(|target| target.slug).collect::<Vec<_>>();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), targets.len());
        assert!(targets.iter().all(|target| target.url.starts_with("https://")));
        assert!(targets.iter().all(|target| !target.company.trim().is_empty()));
    }

    #[test]
    fn prompt_keeps_samples_speculative() {
        let prompt = build_crypto_saas_prompt(&crypto_saas_targets()[0]);
        assert!(prompt.contains("speculative"));
        assert!(prompt.contains("Do not copy trademarks"));
        assert!(prompt.contains("website-to-video"));
    }

    #[test]
    fn extra_payload_carries_source_and_reference() {
        let target = &crypto_saas_targets()[2];
        let extra = build_crypto_saas_extra(target, Some("https://assets.example/hero.png"));
        assert_eq!(extra["portfolio_category"], "crypto_saas");
        assert_eq!(extra["company"], target.company);
        assert_eq!(extra["source_url"], target.url);
        assert_eq!(extra["reference_image_url"], "https://assets.example/hero.png");
        assert_eq!(extra["include_narration"], true);
        assert!(extra["narration_text"].as_str().is_some_and(|text| text.contains(target.company)));
    }

    #[test]
    fn dfy_has_12_services() {
        assert_eq!(dfy_services().len(), 12);
    }

    #[test]
    fn dfy_slugs_are_unique() {
        let mut slugs = dfy_services().iter().map(|s| s.slug).collect::<Vec<_>>();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), dfy_services().len());
    }

    #[test]
    fn dfy_each_has_valid_inputs() {
        for service in dfy_services() {
            let input = service_input_for(service);
            assert!(!input.brief.is_empty());
            assert!(!input.style.is_empty());
            assert!(input.duration_seconds > 0.0);
            // Normalize: the slug must map to a ServiceType that round-trips
            let normalized = crate::services::agentic_service_pipeline::ServiceType::from_normalized(service.slug);
            let normalized_back = crate::services::agentic_service_pipeline::ServiceType::from_normalized(normalized.as_str());
            // Discriminant should be the same (same variant, same ServiceType)
            assert_eq!(
                std::mem::discriminant(&normalized),
                std::mem::discriminant(&normalized_back),
                "slug '{}' normalized to '{}' must round-trip consistently",
                service.slug, normalized.as_str()
            );
        }
    }
}
