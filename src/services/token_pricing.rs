// Token pricing calculation service
// Pricing is hardcoded as fallback but can be overridden from database (system_settings table)
// Last updated: 2025-12-12

use sqlx::PgPool;

/// Hardcoded pricing as fallback (can be overridden from DB)
/// Prices are in USD per million tokens
pub struct ModelPricing {
    pub input_price: f64,
    pub output_price: f64,
    pub input_price_extended: Option<f64>,  // For Claude >200K context
    pub output_price_extended: Option<f64>,
}

impl ModelPricing {
    /// Claude Sonnet 4.5 - Current flagship model
    /// Updated: 2025-12-12
    /// Source: https://claude.com/pricing
    pub fn claude_sonnet_4_5() -> Self {
        Self {
            input_price: 3.00,
            output_price: 15.00,
            input_price_extended: Some(6.00),
            output_price_extended: Some(22.50),
        }
    }

    /// Claude Sonnet 3.5
    /// Updated: 2025-12-12
    pub fn claude_sonnet_3_5() -> Self {
        Self {
            input_price: 3.00,
            output_price: 15.00,
            input_price_extended: None,
            output_price_extended: None,
        }
    }

    /// Gemini 2.0 Flash - Current model
    /// Updated: 2025-12-12
    /// Source: https://ai.google.dev/gemini-api/docs/pricing
    pub fn gemini_2_0_flash() -> Self {
        Self {
            input_price: 0.10,
            output_price: 0.40,
            input_price_extended: None,
            output_price_extended: None,
        }
    }

    /// Gemini 2.5 Flash
    /// Updated: 2025-12-12
    pub fn gemini_2_5_flash() -> Self {
        Self {
            input_price: 0.30,
            output_price: 2.50,
            input_price_extended: None,
            output_price_extended: None,
        }
    }

    /// Calculate cost in USD cents (avoids floating point precision issues)
    /// Returns: (input_cost_cents, output_cost_cents, total_cost_cents)
    pub fn calculate_cost_cents(&self, input_tokens: u32, output_tokens: u32, context_size: u32) -> (i64, i64, i64) {
        // Determine which pricing tier to use (for Claude extended context)
        let use_extended = context_size > 200_000;

        let input_price = if use_extended && self.input_price_extended.is_some() {
            self.input_price_extended.unwrap()
        } else {
            self.input_price
        };

        let output_price = if use_extended && self.output_price_extended.is_some() {
            self.output_price_extended.unwrap()
        } else {
            self.output_price
        };

        // Calculate costs
        let input_cost = (input_tokens as f64 / 1_000_000.0) * input_price * 100.0;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * output_price * 100.0;

        (
            input_cost.round() as i64,
            output_cost.round() as i64,
            (input_cost + output_cost).round() as i64,
        )
    }
}

/// Get pricing for a specific model (tries DB first, falls back to hardcoded)
pub async fn get_model_pricing(
    pool: &PgPool,
    model: &str,
) -> Result<ModelPricing, Box<dyn std::error::Error + Send + Sync>> {
    // Normalize model name
    let model_key = normalize_model_name(model);

    // Try to fetch from database first
    if let Ok(pricing) = fetch_pricing_from_db(pool, &model_key).await {
        return Ok(pricing);
    }

    // Fallback to hardcoded pricing
    Ok(get_hardcoded_pricing(&model_key))
}

/// Fetch pricing from system_settings table
async fn fetch_pricing_from_db(
    pool: &PgPool,
    model_key: &str,
) -> Result<ModelPricing, sqlx::Error> {
    let input_key = format!("model_pricing.{}.input", model_key);
    let output_key = format!("model_pricing.{}.output", model_key);
    let input_base_key = format!("model_pricing.{}.input_base", model_key);
    let output_base_key = format!("model_pricing.{}.output_base", model_key);
    let input_ext_key = format!("model_pricing.{}.input_extended", model_key);
    let output_ext_key = format!("model_pricing.{}.output_extended", model_key);

    // Try base keys first (for Claude 4.5), fallback to simple keys (for others)
    let input_result: Result<(String,), sqlx::Error> = sqlx::query_as(
        "SELECT setting_value FROM system_settings WHERE setting_key = $1 OR setting_key = $2 LIMIT 1"
    )
    .bind(&input_base_key)
    .bind(&input_key)
    .fetch_one(pool)
    .await;

    let output_result: Result<(String,), sqlx::Error> = sqlx::query_as(
        "SELECT setting_value FROM system_settings WHERE setting_key = $1 OR setting_key = $2 LIMIT 1"
    )
    .bind(&output_base_key)
    .bind(&output_key)
    .fetch_one(pool)
    .await;

    let input_ext_result: Result<(String,), sqlx::Error> = sqlx::query_as(
        "SELECT setting_value FROM system_settings WHERE setting_key = $1"
    )
    .bind(&input_ext_key)
    .fetch_optional(pool)
    .await
    .and_then(|opt| opt.ok_or(sqlx::Error::RowNotFound));

    let output_ext_result: Result<(String,), sqlx::Error> = sqlx::query_as(
        "SELECT setting_value FROM system_settings WHERE setting_key = $1"
    )
    .bind(&output_ext_key)
    .fetch_optional(pool)
    .await
    .and_then(|opt| opt.ok_or(sqlx::Error::RowNotFound));

    if let (Ok(input), Ok(output)) = (input_result, output_result) {
        Ok(ModelPricing {
            input_price: input.0.parse().unwrap_or(0.0),
            output_price: output.0.parse().unwrap_or(0.0),
            input_price_extended: input_ext_result.ok().and_then(|r| r.0.parse().ok()),
            output_price_extended: output_ext_result.ok().and_then(|r| r.0.parse().ok()),
        })
    } else {
        Err(sqlx::Error::RowNotFound)
    }
}

/// Get hardcoded pricing as fallback
fn get_hardcoded_pricing(model_key: &str) -> ModelPricing {
    match model_key {
        "claude-sonnet-4-5" | "claude-sonnet-4.5" => ModelPricing::claude_sonnet_4_5(),
        "claude-3-5-sonnet" | "claude-sonnet-3.5" => ModelPricing::claude_sonnet_3_5(),
        "gemini-2.0-flash" | "gemini-2-flash" => ModelPricing::gemini_2_0_flash(),
        "gemini-2.5-flash" => ModelPricing::gemini_2_5_flash(),
        _ => {
            tracing::warn!("Unknown model for pricing: {}, using default", model_key);
            ModelPricing {
                input_price: 0.0,
                output_price: 0.0,
                input_price_extended: None,
                output_price_extended: None,
            }
        }
    }
}

/// Normalize model name to pricing key
fn normalize_model_name(model: &str) -> String {
    if model.contains("claude-sonnet-4") {
        "claude-sonnet-4-5".to_string()
    } else if model.contains("claude") && model.contains("3.5") || model.contains("3-5") {
        "claude-3-5-sonnet".to_string()
    } else if model.contains("gemini-2.5-flash") || model.contains("gemini-flash-2.5") {
        "gemini-2.5-flash".to_string()
    } else if model.contains("gemini-2.0-flash") || model.contains("gemini-2-flash") || model.contains("gemini-flash-2.0") {
        "gemini-2.0-flash".to_string()
    } else {
        model.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_sonnet_4_5_cost_calculation() {
        let pricing = ModelPricing::claude_sonnet_4_5();

        // Small context (≤200K)
        let (input, output, total) = pricing.calculate_cost_cents(5000, 2000, 50000);
        assert_eq!(input, 2);  // (5000/1M) * 3.00 * 100 = 1.5¢ → rounds to 2¢
        assert_eq!(output, 3); // (2000/1M) * 15.00 * 100 = 3¢
        assert_eq!(total, 5);

        // Large context (>200K)
        let (input, output, total) = pricing.calculate_cost_cents(5000, 2000, 250000);
        assert_eq!(input, 3);  // (5000/1M) * 6.00 * 100 = 3¢
        assert_eq!(output, 5); // (2000/1M) * 22.50 * 100 = 4.5¢ → rounds to 5¢
        assert_eq!(total, 8);
    }

    #[test]
    fn test_gemini_flash_cost_calculation() {
        let pricing = ModelPricing::gemini_2_0_flash();

        let (input, output, total) = pricing.calculate_cost_cents(10000, 3000, 0);
        assert_eq!(input, 0);  // (10000/1M) * 0.10 * 100 = 0.1¢ → rounds to 0¢
        assert_eq!(output, 0); // (3000/1M) * 0.40 * 100 = 0.12¢ → rounds to 0¢
        assert_eq!(total, 0);
    }

    #[test]
    fn test_model_name_normalization() {
        assert_eq!(normalize_model_name("claude-sonnet-4-5-20251101"), "claude-sonnet-4-5");
        assert_eq!(normalize_model_name("claude-3-5-sonnet-latest"), "claude-3-5-sonnet");
        assert_eq!(normalize_model_name("gemini-2.0-flash-exp"), "gemini-2.0-flash");
        assert_eq!(normalize_model_name("gemini-2.5-flash"), "gemini-2.5-flash");
    }
}
