//! Portfolio test runner — Managed Campaign service scenarios via AgenticServicePipeline.
//!
//! Triggered from /admin/test-runs. Each run exercises all 12 Managed Campaign services
//! through the AgenticServicePipeline (same pipeline prospects use).
//! Results are stored in `test_runs` / `test_results` tables and surfaced
//! in the admin dashboard for manual quality review.

use crate::AppState;
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

struct LLMReview {
    score: i32,
    feedback: String,
}

fn parse_review(text: &str) -> LLMReview {
    let trimmed = text.trim();
    let json_str = if let (Some(s), Some(e)) = (trimmed.find('{'), trimmed.rfind('}')) {
        &trimmed[s..=e]
    } else {
        trimmed
    };
    if let Ok(val) = serde_json::from_str::<Value>(json_str) {
        let score = val.get("score").and_then(|v| v.as_i64()).unwrap_or(5) as i32;
        let feedback = val
            .get("feedback")
            .and_then(|v| v.as_str())
            .unwrap_or("No feedback provided")
            .to_string();
        return LLMReview {
            score: score.clamp(1, 10),
            feedback,
        };
    }
    LLMReview {
        score: 5,
        feedback: trimmed.chars().take(500).collect(),
    }
}

struct DfyScenario {
    slug: &'static str,
    name: &'static str,
    description: &'static str,
}

fn scenarios() -> Vec<DfyScenario> {
    crate::portfolio_samples::dfy_services().iter().map(|svc| {
        DfyScenario {
            slug: svc.slug,
            name: svc.name,
            description: svc.brief,
        }
    }).collect()
}

// ── Runner ────────────────────────────────────────────────────────────────

pub struct PortfolioTestRunner {
    app_state: Arc<AppState>,
}

impl PortfolioTestRunner {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Create a new test_run row and spawn a background task that runs all 12 Managed Campaign services.
    pub async fn create_and_spawn(app_state: Arc<AppState>, name: String) -> Result<Uuid, String> {
        let row = sqlx::query("INSERT INTO test_runs (name) VALUES ($1) RETURNING id")
            .bind(&name)
            .fetch_one(&app_state.db_pool)
            .await
            .map_err(|e| format!("Failed to create test run: {e}"))?;

        let run_id: Uuid = row.get("id");

        tokio::spawn(async move {
            let runner = PortfolioTestRunner::new(app_state);
            runner.run_all(run_id).await;
        });

        Ok(run_id)
    }

    async fn run_all(&self, run_id: Uuid) {
        let all = scenarios();
        let total = all.len() as i32;

        let _ = sqlx::query("UPDATE test_runs SET total_tests = $1 WHERE id = $2")
            .bind(total)
            .bind(run_id)
            .execute(&self.app_state.db_pool)
            .await;

        for scenario in &all {
            self.run_one(run_id, scenario).await;
            // Brief pause between scenarios
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }

        let _ = sqlx::query(
            "UPDATE test_runs \
             SET status = CASE WHEN failed_tests = 0 THEN 'completed' ELSE 'completed_with_failures' END, \
                 completed_at = NOW() \
             WHERE id = $1",
        )
        .bind(run_id)
        .execute(&self.app_state.db_pool)
        .await;

        tracing::info!("Portfolio test run {run_id} finished (12 Managed Campaign services)");
    }

    async fn run_one(&self, run_id: Uuid, scenario: &DfyScenario) {
        tracing::info!("▶ Managed Campaign test: {}", scenario.name);

        let result_id = match sqlx::query(
            "INSERT INTO test_results (run_id, test_name, gig_type, prompt, status) \
             VALUES ($1, $2, $3, $4, 'running') RETURNING id",
        )
        .bind(run_id)
        .bind(scenario.name)
        .bind(scenario.slug)
        .bind(scenario.description)
        .fetch_one(&self.app_state.db_pool)
        .await
        {
            Ok(r) => { let id: Uuid = r.get("id"); id }
            Err(e) => {
                tracing::error!("DB insert failed for '{}': {e}", scenario.name);
                return;
            }
        };

        match self.execute_via_pipeline(scenario).await {
            Ok((r2_url, filename)) => {
                let review = self.review(&r2_url, scenario).await;

                let _ = sqlx::query(
                    "UPDATE test_results SET \
                         status = 'passed', \
                         output_r2_key = $1, \
                         output_r2_url = $2, \
                         output_filename = $3, \
                         llm_review_score = $4, \
                         llm_review_feedback = $5, \
                         llm_reviewer = 'gemini', \
                         completed_at = NOW() \
                     WHERE id = $6",
                )
                .bind(&r2_url)
                .bind(&r2_url)
                .bind(&filename)
                .bind(review.score)
                .bind(&review.feedback)
                .bind(result_id)
                .execute(&self.app_state.db_pool)
                .await;

                let _ = sqlx::query(
                    "UPDATE test_runs SET passed_tests = passed_tests + 1 WHERE id = $1",
                )
                .bind(run_id)
                .execute(&self.app_state.db_pool)
                .await;

                tracing::info!("✅ '{}' passed — score {}/10", scenario.name, review.score);
            }
            Err(e) => {
                tracing::error!("❌ '{}' failed: {e}", scenario.name);

                let _ = sqlx::query(
                    "UPDATE test_results SET \
                         status = 'failed', \
                         error_message = $1, \
                         completed_at = NOW() \
                     WHERE id = $2",
                )
                .bind(&e)
                .bind(result_id)
                .execute(&self.app_state.db_pool)
                .await;

                let _ = sqlx::query(
                    "UPDATE test_runs SET failed_tests = failed_tests + 1 WHERE id = $1",
                )
                .bind(run_id)
                .execute(&self.app_state.db_pool)
                .await;
            }
        }
    }

    /// Run a Managed Campaign service through the AgenticServicePipeline and wait for completion.
    /// Returns (output_r2_url, filename) on success.
    async fn execute_via_pipeline(&self, scenario: &DfyScenario) -> Result<(String, String), String> {
        let service_type = crate::services::agentic_service_pipeline::ServiceType::from_normalized(scenario.slug);
        let delivery_id = Uuid::new_v4();

        // Create a delivery row
        let service_def = crate::portfolio_samples::dfy_services()
            .iter()
            .find(|s| s.slug == scenario.slug)
            .ok_or_else(|| format!("Managed Campaign service '{}' not found", scenario.slug))?;

        let input = crate::portfolio_samples::service_input_for(service_def);
        let mut extra = crate::portfolio_samples::portfolio_extra_args(service_def);
        extra["test_run_id"] = json!(delivery_id.to_string());

        let title = format!("Test Run — {}", scenario.name);

        let _ = sqlx::query(
            "INSERT INTO deliveries (id, client_ref, title, gig_type, prompt, style, duration, extra_args, status, unlock_price_usdc, source_url) \
             VALUES ($1, $2, $3, 'agentic_pipeline', $4, $5, $6, $7, 'pending', 0.0, $8)",
        )
        .bind(delivery_id)
        .bind(format!("test-run:{}", scenario.slug))
        .bind(&title)
        .bind(&input.brief)
        .bind(&input.style)
        .bind(input.duration_seconds)
        .bind(&extra)
        .bind(&input.source_url)
        .execute(&self.app_state.db_pool)
        .await
        .map_err(|e| format!("DB insert failed: {e}"))?;

        // Start the AgenticServicePipeline
        let svc_input = crate::services::agentic_service_pipeline::ServiceInput {
            title,
            brief: input.brief.clone(),
            source_url: input.source_url.clone(),
            style: input.style.clone(),
            duration_seconds: input.duration_seconds,
            delivery_id,
            prospect_id: None,
            session_uuid: None,
            user_id: None,
            source_table: Some("deliveries".to_string()),
            source_record_id: Some(delivery_id),
            idempotency_key: Some(format!("test-run:{}", scenario.slug)),
            reference_images: vec![],
        };

        let workflow_id = crate::services::AgenticServicePipeline::start(
            self.app_state.clone(),
            service_type,
            svc_input,
        )
        .await
        .map_err(|e| format!("AgenticServicePipeline start failed: {e}"))?;

        let _ = sqlx::query("UPDATE deliveries SET workflow_id = $1 WHERE id = $2")
            .bind(workflow_id)
            .bind(delivery_id)
            .execute(&self.app_state.db_pool)
            .await;

        // Poll for completion (up to 10 minutes)
        let max_polls = 120u32; // 120 * 5s = 600s = 10 minutes
        for _ in 0..max_polls {
            let row = sqlx::query(
                "SELECT status, output_r2_url, error_message FROM deliveries WHERE id = $1",
            )
            .bind(delivery_id)
            .fetch_optional(&self.app_state.db_pool)
            .await
            .map_err(|e| format!("DB poll failed: {e}"))?
            .ok_or_else(|| "Delivery row vanished".to_string())?;

            let status: String = row.get("status");
            match status.as_str() {
                "completed" => {
                    let url: Option<String> = row.get("output_r2_url");
                    match url {
                        Some(u) if !u.trim().is_empty() => {
                            let filename = format!("test-run-{}.mp4", scenario.slug);
                            return Ok((u, filename));
                        }
                        _ => return Err("Delivery completed but no output URL".to_string()),
                    }
                }
                "failed" => {
                    let err: Option<String> = row.get("error_message");
                    return Err(err.unwrap_or_else(|| "Delivery failed with no error message".to_string()));
                }
                _ => {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }

        Err("Delivery timed out (10 min) — pipeline may be stuck or queue is too deep".to_string())
    }

    async fn review(&self, r2_url: &str, scenario: &DfyScenario) -> LLMReview {
        let gemini = self.app_state.video_gemini_client.as_ref()
            .or(self.app_state.gemini_client.as_ref());

        let Some(client) = gemini else {
            return LLMReview { score: 0, feedback: "Gemini not configured — review skipped".to_string() };
        };

        let prompt = format!(
            "You are reviewing a Managed Campaign demo video generated by an AI agent pipeline.\n\
             Service: {name} ({slug})\n\
             Brief: \"{desc}\"\n\
             Pipeline: AgenticServicePipeline with StatefulGeminiAgent (full ~223 tool catalog)\n\n\
             The video is hosted at: {url}\n\n\
             Rate this video 1-10 (10=perfect) on:\n\
             - Visual quality and production value\n\
             - How well it sells the Managed Campaign service to a prospect\n\
             - Would this be good enough to send in a cold DM/email?\n\n\
             Reply ONLY as JSON: {{\"score\": <int 1-10>, \"feedback\": \"<2 sentences max>\"}}",
            name = scenario.name,
            slug = scenario.slug,
            desc = scenario.description,
            url = r2_url,
        );

        match client.generate_text(&prompt).await {
            Ok(resp) => parse_review(&resp),
            Err(e) => LLMReview { score: 0, feedback: format!("Review call failed: {e}") },
        }
    }
}
