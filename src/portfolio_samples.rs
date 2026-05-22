use serde::Serialize;
use serde_json::{json, Value};

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
        PortfolioTarget {
            slug: "privy",
            company: "Privy",
            url: "https://www.privy.io/",
            market: "wallet infrastructure",
            angle: "embedded wallets, programmable keys, and secure user onboarding",
            visual_direction: "premium wallet-stack command center with secure key shards, passkey login, and flowing transaction rails",
        },
        PortfolioTarget {
            slug: "dynamic",
            company: "Dynamic",
            url: "https://www.dynamic.xyz/",
            market: "wallet infrastructure for fintech and stablecoin apps",
            angle: "sub-second signing, embedded wallets, and revenue-ready crypto features",
            visual_direction: "enterprise fintech dashboard with live wallet activity, stablecoin motion trails, and fast-signing status pulses",
        },
        PortfolioTarget {
            slug: "crossmint",
            company: "Crossmint",
            url: "https://www.crossmint.com/",
            market: "stablecoin and wallet infrastructure",
            angle: "wallets, onramps, stablecoin orchestration, checkout, and tokenization APIs",
            visual_direction: "clean orchestration map with wallets, onramps, stablecoin transfers, and agent treasury nodes",
        },
        PortfolioTarget {
            slug: "thirdweb",
            company: "thirdweb",
            url: "https://thirdweb.com/",
            market: "web3 developer and agent infrastructure",
            angle: "AI agents, native internet payments, wallets, contracts, and blockchain data access",
            visual_direction: "developer launch bay with agent wallets, contract APIs, payment streams, and blockchain data panels",
        },
        PortfolioTarget {
            slug: "coinbase-business",
            company: "Coinbase Business",
            url: "https://www.coinbase.com/commerce",
            market: "crypto business payments",
            angle: "global payments, business accounts, API payouts, and USDC treasury workflows",
            visual_direction: "modern business finance workspace with global payment arcs, USDC treasury cards, and checkout confirmations",
        },
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
        company = target.company,
        url = target.url,
        market = target.market,
        angle = target.angle,
    )
}

pub fn build_crypto_saas_extra(
    target: &PortfolioTarget,
    reference_image_url: Option<&str>,
) -> Value {
    let narration_text = format!(
        "{company} powers {market}. This speculative VideoSync sample shows {angle} in a polished website-to-video explainer built for outbound sales.",
        company = target.company,
        market = target.market,
        angle = target.angle,
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
        assert!(targets
            .iter()
            .all(|target| target.url.starts_with("https://")));
        assert!(targets
            .iter()
            .all(|target| !target.company.trim().is_empty()));
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
        assert_eq!(
            extra["reference_image_url"],
            "https://assets.example/hero.png"
        );
        assert_eq!(extra["include_narration"], true);
        assert!(extra["narration_text"]
            .as_str()
            .is_some_and(|text| text.contains(target.company)));
    }
}
