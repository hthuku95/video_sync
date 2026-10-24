//! Runtime on/off switches for customer-facing payment methods (owner directive Oct 2026).
//!
//! Rows live in the `payment_methods` table (see migration
//! `20261024000003_payment_methods.sql`) and flip at runtime via the admin UI or
//! `POST /api/admin/payment-methods`. Missing rows default to disabled so a
//! freshly-deployed method never appears until explicitly enabled.

pub const KNOWN_METHODS: &[&str] = &["paypal", "usdc"];

pub fn is_known_method(method: &str) -> bool {
    let m = method.to_ascii_lowercase();
    KNOWN_METHODS.contains(&m.as_str())
}

pub async fn method_enabled(pool: &sqlx::PgPool, method: &str) -> bool {
    let key = method.to_ascii_lowercase();
    let row: Option<(bool,)> =
        sqlx::query_as("SELECT enabled FROM payment_methods WHERE method = $1")
            .bind(&key)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);
    row.map(|(enabled,)| enabled).unwrap_or(false)
}

pub async fn enabled_methods(pool: &sqlx::PgPool) -> Vec<String> {
    let mut out = Vec::new();
    for m in KNOWN_METHODS {
        if method_enabled(pool, m).await {
            out.push(m.to_string());
        }
    }
    out
}
