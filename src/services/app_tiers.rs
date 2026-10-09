//! Per-user per-app agency tiers (owner directive Oct 2026).
//!
//! Model: one subscription per app per user; each tier carries an account cap
//! (total connected Zernio accounts across ALL the user's profiles — caps are
//! user-level, never per-campaign, so N campaigns can't multiply exposure).
//! Only 4 platforms may be connected (YouTube, TikTok, Instagram, Facebook).
//!
//! Tiers: base (20 accounts, app price) | agency50 (50, $499/mo) |
//! agency150 (150, $999/mo). Missing user_app_subscriptions row = base.

use std::sync::Arc;

use crate::AppState;

/// Platforms users may connect (owner directive Oct 2026). Everything else is
/// refused at connect time; pre-existing others keep working but can't grow.
pub const ALLOWED_PLATFORMS: &[&str] = &["youtube", "tiktok", "instagram", "facebook"];

pub fn tier_cap(tier: &str) -> usize {
    match tier {
        "agency150" => 150,
        "agency50" => 50,
        _ => 20,
    }
}

/// Monthly price in cents. Base follows the app subscription; agency tiers
/// are flat per app.
pub fn tier_price_cents(app: &str, tier: &str) -> u64 {
    match tier {
        "agency50" => 49900,
        "agency150" => 99900,
        _ => crate::handlers::campaigns::campaign_app_price_cents(app),
    }
}

pub fn is_known_tier(tier: &str) -> bool {
    matches!(tier, "base" | "agency50" | "agency150")
}

/// Effective tier for a user+app (missing row = base/active).
pub async fn get_user_tier(pool: &sqlx::PgPool, user_id: i32, app: &str) -> (String, usize) {
    let tier: Option<String> = sqlx::query_scalar(
        "SELECT tier FROM user_app_subscriptions WHERE user_id = $1 AND app_slug = $2 AND status = 'active'",
    )
    .bind(user_id)
    .bind(app)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let tier = tier.unwrap_or_else(|| "base".to_string());
    let cap = tier_cap(&tier);
    (tier, cap)
}

/// Count the user's DISTINCT active connected accounts across ALL their Zernio
/// profiles (live API; falls back to the DB cache when Zernio is unreachable).
pub async fn count_user_accounts(state: &Arc<AppState>, user_id: i32) -> usize {
    let profiles: Vec<String> = sqlx::query_scalar(
        "SELECT zernio_profile_id FROM user_zernio_profiles WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();
    if profiles.is_empty() {
        return 0;
    }
    if let Some(z) = state.zernio_client.clone() {
        let mut seen = std::collections::HashSet::new();
        for pid in &profiles {
            if let Ok(resp) = z.list_accounts(Some(pid)).await {
                for a in resp.accounts.iter().filter(|a| a.is_active) {
                    seen.insert(a.id.clone());
                }
            }
        }
        if !seen.is_empty() || profiles.len() == 1 {
            return seen.len();
        }
    }
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(DISTINCT zernio_account_id) FROM user_zernio_accounts WHERE user_id = $1 AND is_active = TRUE",
    )
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(0) as usize
}
