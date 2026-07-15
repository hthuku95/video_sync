//! x402 payment protocol — HTTP 402 Payment Required for crypto payments.
//!
//! This module implements the **server side** of the x402 protocol launched
//! by Coinbase + Linux Foundation in 2026. Buyers sign an EIP-3009 USDC
//! `transferWithAuthorization` off-chain in their wallet (Phantom, MetaMask,
//! Coinbase Wallet, etc.); we relay that signed authorization to the
//! Coinbase facilitator, which submits it on-chain on Base, and we get a
//! receipt id (transaction hash) we can persist.
//!
//! Why this matters for the platform:
//! * No Stripe account needed — the user explicitly didn't have one.
//! * Phantom wallet (which the user already has) supports EVM signing on
//!   Base as of their multi-chain update.
//! * Campaign and subscription activation via x402 USDC on Base.
//!   Used for campaign pay-spec/settle flow and /subscribe payments.
//!
//! Reference:
//!   <https://github.com/coinbase/x402/blob/main/specs/schemes/exact/scheme_exact_evm.md>
//!   <https://docs.cdp.coinbase.com/x402/welcome>

use serde::{Deserialize, Serialize};

/// USDC contract address on Base mainnet. Hardcoded because we only support
/// USDC (the x402 reference asset). To support other assets, switch to a
/// per-PaymentRequirements `asset` field.
pub const USDC_BASE_MAINNET: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";

/// The `scheme` we use — `exact` means "transfer this exact amount". The
/// other scheme in the spec (`upto`) is for streaming/usage-based pricing.
const SCHEME: &str = "exact";

/// Network identifier used by the facilitator. `base` = Base mainnet.
/// Switch to `base-sepolia` for testnet during integration testing.
const DEFAULT_NETWORK: &str = "base";

/// 5-minute payment validity window. Long enough that a buyer can sign in
/// their wallet and submit, short enough that an old signed authorization
/// can't be replayed days later if the wallet is later compromised.
const PAYMENT_VALIDITY_SECONDS: i64 = 300;

/// What we send back in the body of an HTTP 402 response.
///
/// Layout matches the x402 reference spec — clients (Coinbase wallet,
/// any x402-aware library) parse this, sign EIP-3009 over the
/// `transferWithAuthorization` struct, then retry the original request
/// with the `X-Payment` header populated.
#[derive(Debug, Clone, Serialize)]
pub struct PaymentRequiredResponse {
    /// Always 1 currently.
    pub x402_version: u32,
    /// One entry per accepted payment option. We only emit one (USDC/Base).
    pub accepts: Vec<PaymentRequirement>,
    /// Human-readable explanation rendered by the wallet UI.
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentRequirement {
    pub scheme: String,
    pub network: String,
    /// Max accepted amount, in atomic units (USDC has 6 decimals — $5 = 5_000_000).
    #[serde(rename = "maxAmountRequired")]
    pub max_amount_required: String,
    /// Resource being paid for (your URL).
    pub resource: String,
    /// Free-form description the wallet shows.
    pub description: String,
    /// Mime type of the eventual response.
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    /// Recipient wallet address (where USDC arrives).
    #[serde(rename = "payTo")]
    pub pay_to: String,
    /// How long the buyer has to sign + submit (seconds).
    #[serde(rename = "maxTimeoutSeconds")]
    pub max_timeout_seconds: i64,
    /// USDC contract address on the chosen network.
    pub asset: String,
    /// Optional EIP-712 domain extras. For USDC on Base this is
    /// `{"name":"USD Coin","version":"2"}` per the contract ABI.
    pub extra: serde_json::Value,
}

/// Build a 402 response for a fixed price.
///
/// * `price_usd_cents` — e.g. 500 for $5.00. Converted to USDC's 6-decimal
///   atomic unit internally (500 cents = 5_000_000 USDC atomic).
/// * `recipient` — your wallet address (env var `X402_RECIPIENT_ADDRESS`).
/// * `resource_url` — the absolute URL of the gated endpoint.
/// * `description` — shown by the wallet (e.g. "Unlock HD download").
pub fn build_payment_required(
    price_usd_cents: u64,
    recipient: &str,
    resource_url: &str,
    description: &str,
) -> PaymentRequiredResponse {
    // USDC has 6 decimals → cents (10^-2) become 10^-6 by multiplying by 10^4.
    let atomic = (price_usd_cents as u128) * 10_000;

    let network = std::env::var("X402_NETWORK").unwrap_or_else(|_| DEFAULT_NETWORK.to_string());

    let req = PaymentRequirement {
        scheme: SCHEME.to_string(),
        network: network.clone(),
        max_amount_required: atomic.to_string(),
        resource: resource_url.to_string(),
        description: description.to_string(),
        mime_type: "application/json".to_string(),
        pay_to: recipient.to_string(),
        max_timeout_seconds: PAYMENT_VALIDITY_SECONDS,
        asset: std::env::var("X402_ASSET_ADDRESS")
            .unwrap_or_else(|_| USDC_BASE_MAINNET.to_string()),
        extra: serde_json::json!({"name": "USD Coin", "version": "2"}),
    };

    PaymentRequiredResponse {
        x402_version: 1,
        accepts: vec![req],
        error: format!(
            "Payment required: ${:.2} USDC on {}",
            price_usd_cents as f64 / 100.0,
            network
        ),
    }
}

/// Verification result returned by the facilitator.
#[derive(Debug, Clone, Deserialize)]
pub struct VerifyResponse {
    /// Whether the payment is valid + settled (or eligible for settlement).
    #[serde(rename = "isValid")]
    pub is_valid: bool,
    /// Reason if invalid.
    #[serde(rename = "invalidReason")]
    pub invalid_reason: Option<String>,
    /// On-chain transaction hash once submitted. Persist this as receipt.
    #[serde(rename = "txHash")]
    pub tx_hash: Option<String>,
    /// Sender address (lifted from the signed authorization).
    #[allow(dead_code)]
    pub payer: Option<String>,
}

/// Verify + settle a payment via the Coinbase facilitator.
///
/// The facilitator is a hosted service that:
///   1. Decodes the base64 `X-Payment` header.
///   2. Validates the EIP-712 signature against the EIP-3009 struct.
///   3. Submits `transferWithAuthorization` on-chain to the USDC contract.
///   4. Returns the tx hash.
///
/// We pass the same `requirements` we originally sent in the 402 so the
/// facilitator can sanity-check the sig matches what we're charging for.
///
/// `X402_FACILITATOR_URL` defaults to the public free endpoint; switch to
/// the Coinbase CDP facilitator for higher rate limits.
pub async fn verify_payment(
    x_payment_header_b64: &str,
    requirements: &PaymentRequirement,
) -> Result<VerifyResponse, String> {
    let payload_json: serde_json::Value = {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(x_payment_header_b64)
            .map_err(|e| format!("X-Payment is not valid base64: {}", e))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| format!("X-Payment payload is not JSON: {}", e))?
    };

    let facilitator_url = std::env::var("X402_FACILITATOR_URL")
        .unwrap_or_else(|_| "https://x402.org/facilitator".to_string());
    let facilitator_token = std::env::var("X402_FACILITATOR_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());

    if facilitator_url == "https://x402.org/facilitator" && requirements.network == "base" {
        return Err(
            "Production Base payments require a mainnet facilitator. Configure X402_FACILITATOR_URL and X402_FACILITATOR_TOKEN for the Coinbase CDP facilitator instead of the free x402.org test facilitator.".to_string()
        );
    }

    let body = serde_json::json!({
        "x402Version":         1,
        "paymentPayload":      payload_json,
        "paymentRequirements": requirements,
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(|e| format!("HTTP client init failed: {}", e))?;

    // The verify endpoint validates the signature and the settle endpoint
    // submits on-chain. Coinbase's facilitator exposes both at /verify and
    // /settle; for public x402.org, /settle does both in one call. We use
    // /settle so a buyer's payment is final by the time we return 200.
    let mut request = client.post(format!("{}/settle", facilitator_url)).json(&body);
    if let Some(token) = facilitator_token {
        request = request.bearer_auth(token);
    }

    let resp = request
        .send()
        .await
        .map_err(|e| format!("Facilitator request failed: {}", e))?;

    let status = resp.status();
    let raw = resp
        .text()
        .await
        .map_err(|e| format!("Facilitator response read failed: {}", e))?;

    if !status.is_success() {
        return Err(format!("Facilitator HTTP {}: {}", status, raw));
    }

    serde_json::from_str::<VerifyResponse>(&raw)
        .map_err(|e| format!("Facilitator response parse failed: {} — body: {}", e, raw))
}

/// Convenience wrapper used by route handlers — returns Ok(tx_hash) when the
/// payment is verified and settled, Err with a human-readable reason
/// otherwise.
pub async fn settle_or_reject(
    x_payment_header_b64: &str,
    requirements: &PaymentRequirement,
) -> Result<String, String> {
    let res = verify_payment(x_payment_header_b64, requirements).await?;
    if !res.is_valid {
        return Err(res
            .invalid_reason
            .unwrap_or_else(|| "Payment rejected by facilitator".to_string()));
    }
    res.tx_hash
        .ok_or_else(|| "Facilitator returned no tx_hash despite isValid=true".to_string())
}
