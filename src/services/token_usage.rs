// Token usage tracking and cost recording service
// Records API token usage and costs to database for billing and analytics

use sqlx::PgPool;
use super::token_pricing::{get_model_pricing, ModelPricing};

pub struct TokenUsageService;

impl TokenUsageService {
    /// Record Claude API token usage and cost
    pub async fn record_claude_usage(
        pool: &PgPool,
        session_id: i32,
        user_id: i32,
        message_id: Option<i32>,
        job_id: Option<&str>,
        model: &str,
        request_type: &str,
        input_tokens: u32,
        output_tokens: u32,
        context_size: u32,
        cache_creation_tokens: Option<u32>,
        cache_read_tokens: Option<u32>,
    ) -> Result<i64, sqlx::Error> {
        // Get pricing (try DB first, fallback to hardcoded)
        let pricing = get_model_pricing(pool, model).await
            .unwrap_or_else(|_| ModelPricing::claude_sonnet_4_5());

        let (input_cost, output_cost, _) = pricing.calculate_cost_cents(
            input_tokens,
            output_tokens,
            context_size,
        );

        let result: (i32,) = sqlx::query_as(
            r#"
            INSERT INTO api_token_usage (
                session_id, user_id, message_id, job_id,
                provider, model, request_type,
                input_tokens, output_tokens,
                input_cost_cents, output_cost_cents,
                cache_creation_tokens, cache_read_tokens, context_size
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            RETURNING id
            "#
        )
        .bind(session_id)
        .bind(user_id)
        .bind(message_id)
        .bind(job_id)
        .bind("claude")
        .bind(model)
        .bind(request_type)
        .bind(input_tokens as i32)
        .bind(output_tokens as i32)
        .bind(input_cost)
        .bind(output_cost)
        .bind(cache_creation_tokens.map(|t| t as i32))
        .bind(cache_read_tokens.map(|t| t as i32))
        .bind(context_size as i32)
        .fetch_one(pool)
        .await?;

        tracing::debug!(
            "💰 Recorded Claude usage: {} input, {} output tokens = ${:.4}",
            input_tokens,
            output_tokens,
            (input_cost + output_cost) as f64 / 100.0
        );

        Ok(result.0 as i64)
    }

    /// Record Gemini API token usage and cost
    pub async fn record_gemini_usage(
        pool: &PgPool,
        session_id: i32,
        user_id: i32,
        message_id: Option<i32>,
        job_id: Option<&str>,
        model: &str,
        request_type: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) -> Result<i64, sqlx::Error> {
        // Get pricing (try DB first, fallback to hardcoded)
        let pricing = get_model_pricing(pool, model).await
            .unwrap_or_else(|_| ModelPricing::gemini_2_0_flash());

        let (input_cost, output_cost, _) = pricing.calculate_cost_cents(input_tokens, output_tokens, 0);

        let result: (i32,) = sqlx::query_as(
            r#"
            INSERT INTO api_token_usage (
                session_id, user_id, message_id, job_id,
                provider, model, request_type,
                input_tokens, output_tokens,
                input_cost_cents, output_cost_cents
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            RETURNING id
            "#
        )
        .bind(session_id)
        .bind(user_id)
        .bind(message_id)
        .bind(job_id)
        .bind("gemini")
        .bind(model)
        .bind(request_type)
        .bind(input_tokens as i32)
        .bind(output_tokens as i32)
        .bind(input_cost)
        .bind(output_cost)
        .fetch_one(pool)
        .await?;

        tracing::debug!(
            "💰 Recorded Gemini usage: {} input, {} output tokens = ${:.4}",
            input_tokens,
            output_tokens,
            (input_cost + output_cost) as f64 / 100.0
        );

        Ok(result.0 as i64)
    }

    /// Get total usage and cost for a user
    pub async fn get_user_total_cost(
        pool: &PgPool,
        user_id: i32,
    ) -> Result<UserUsageSummary, sqlx::Error> {
        let result: (i64, i64, i64, i64) = sqlx::query_as(
            r#"
            SELECT
                COALESCE(SUM(input_tokens), 0) as total_input,
                COALESCE(SUM(output_tokens), 0) as total_output,
                COALESCE(SUM(total_cost_cents), 0) as total_cost_cents,
                COUNT(*) as total_requests
            FROM api_token_usage t
            JOIN chat_sessions s ON s.id = t.session_id
            WHERE s.user_id = $1
            "#
        )
        .bind(user_id)
        .fetch_one(pool)
        .await?;

        Ok(UserUsageSummary {
            total_input_tokens: result.0,
            total_output_tokens: result.1,
            total_tokens: result.0 + result.1,
            total_cost_usd: result.2 as f64 / 100.0,
            total_requests: result.3,
        })
    }

    /// Get usage and cost for a specific session
    pub async fn get_session_cost(
        pool: &PgPool,
        session_id: i32,
    ) -> Result<SessionUsageSummary, sqlx::Error> {
        let result: (i64, i64, i64, i64) = sqlx::query_as(
            r#"
            SELECT
                COALESCE(SUM(input_tokens), 0) as total_input,
                COALESCE(SUM(output_tokens), 0) as total_output,
                COALESCE(SUM(total_cost_cents), 0) as total_cost_cents,
                COUNT(*) as total_requests
            FROM api_token_usage
            WHERE session_id = $1
            "#
        )
        .bind(session_id)
        .fetch_one(pool)
        .await?;

        Ok(SessionUsageSummary {
            total_input_tokens: result.0,
            total_output_tokens: result.1,
            total_tokens: result.0 + result.1,
            total_cost_usd: result.2 as f64 / 100.0,
            total_requests: result.3,
        })
    }

    /// Get usage breakdown by provider
    pub async fn get_usage_by_provider(
        pool: &PgPool,
        user_id: i32,
    ) -> Result<Vec<ProviderUsage>, sqlx::Error> {
        let results: Vec<(String, String, i64, i64, i64, i64)> = sqlx::query_as(
            r#"
            SELECT
                t.provider,
                t.model,
                COUNT(*) as requests,
                COALESCE(SUM(t.input_tokens), 0) as input_tokens,
                COALESCE(SUM(t.output_tokens), 0) as output_tokens,
                COALESCE(SUM(t.total_cost_cents), 0) as cost_cents
            FROM api_token_usage t
            JOIN chat_sessions s ON s.id = t.session_id
            WHERE s.user_id = $1
            GROUP BY t.provider, t.model
            ORDER BY cost_cents DESC
            "#
        )
        .bind(user_id)
        .fetch_all(pool)
        .await?;

        Ok(results
            .into_iter()
            .map(|r| ProviderUsage {
                provider: r.0,
                model: r.1,
                total_requests: r.2,
                total_input_tokens: r.3,
                total_output_tokens: r.4,
                total_cost_usd: r.5 as f64 / 100.0,
            })
            .collect())
    }
}

/// User usage summary
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserUsageSummary {
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
    pub total_requests: i64,
}

/// Session usage summary
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionUsageSummary {
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
    pub total_requests: i64,
}

/// Provider usage breakdown
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderUsage {
    pub provider: String,
    pub model: String,
    pub total_requests: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_usd: f64,
}
