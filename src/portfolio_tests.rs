//! Portfolio test runner — Managed Campaign service scenarios via AgenticServicePipeline.
//!
//! Triggered from /admin/test-runs. Each run exercises all 12 Managed Campaign services
//! through the AgenticServicePipeline (same pipeline prospects use).
//! Results are stored in `test_runs` / `test_results` tables and surfaced
//! in the admin dashboard for manual quality review.

use crate::AppState;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// How often the task driving a run pings `test_runs.heartbeat_at`.
const RUN_HEARTBEAT_SECS: u64 = 20;

/// A running run whose heartbeat is older than this has no live driver.
/// Comfortably larger than RUN_HEARTBEAT_SECS so a busy replica is never
/// mistaken for a dead one, and small enough that a deploy resumes promptly.
/// Neon occasionally drops several writes in a row (SSL EOF under load), and a
/// false resume is expensive: it re-renders and double-counts. Five minutes
/// still recovers a genuinely dead driver quickly.
const RUN_STALE_AFTER_SECS: i64 = 300;

/// How often every replica looks for orphaned runs.
const RUN_RECONCILE_SECS: u64 = 60;

/// Aborts the heartbeat task when the run future ends — including on panic.
/// Dropping a JoinHandle does NOT abort it, so this guard is required; without
/// it a panicking runner would leave a heartbeat pinging forever and the run
/// could never be detected as orphaned.
struct HeartbeatGuard(tokio::task::JoinHandle<()>);

impl Drop for HeartbeatGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct LLMReview {
    score: i32,
    feedback: String,
    /// Which provider actually produced the score, so the report does not claim
    /// "reviewed by gemini" when Gemini 429'd and NIM answered.
    reviewer: String,
}

/// The delivery backing one scenario, plus the input needed to start its
/// pipeline. Rebuilt from the DB when a run resumes.
struct DeliveryPlan {
    delivery_id: Uuid,
    input: crate::services::agentic_service_pipeline::ServiceInput,
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

    /// Create a new test_run row and spawn a background task that runs all enabled scenarios.
    pub async fn create_and_spawn(app_state: Arc<AppState>, name: String) -> Result<Uuid, String> {
        // Serialise runs. The agentic pool is one worker per Fargate task and a
        // clipping delivery legitimately runs for ~an hour, so a second run does
        // not slow the first down — it starves it, and both report timeouts that
        // look like product failures. (2026-10-20: five concurrent runs left
        // five clipping workflows queued behind three workers.)
        let active = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM test_runs \
              WHERE status = 'running' \
                AND COALESCE(heartbeat_at, started_at) > NOW() - INTERVAL '2 minutes' \
              ORDER BY started_at DESC LIMIT 1",
        )
        .fetch_optional(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to check for an active test run: {e}"))?;

        if let Some(active_id) = active {
            return Err(format!(
                "Test run {active_id} is already active. Wait for it to finish or cancel it before starting another."
            ));
        }

        let row = sqlx::query(
            "INSERT INTO test_runs (name, heartbeat_at) VALUES ($1, NOW()) RETURNING id",
        )
        .bind(&name)
        .fetch_one(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to create test run: {e}"))?;

        let run_id: Uuid = row.get("id");

        Self::spawn_run(app_state, run_id, false);

        Ok(run_id)
    }

    /// Spawn the driver task. `resume = true` reattaches an interrupted run
    /// instead of starting the scenarios over.
    fn spawn_run(app_state: Arc<AppState>, run_id: Uuid, resume: bool) {
        tokio::spawn(async move {
            let runner = PortfolioTestRunner::new(app_state);
            runner.run_all(run_id, resume).await;
        });
    }

    async fn run_all(&self, run_id: Uuid, resume: bool) {
        // Ownership first. The reconciler resumes on a stale heartbeat, and a
        // false positive used to start a second driver for a live run.
        let Some(token) = self.claim_driver(run_id).await else {
            tracing::warn!(
                "⏹️ Test run {} already has a live driver — duplicate spawn exiting",
                run_id
            );
            return;
        };

        // Liveness: this task is the only thing that can drive the run. If it
        // dies the timestamp stops advancing and another replica takes over.
        let _heartbeat = self.start_heartbeat(run_id, token);

        // Service switch (owner directive Sep 2026): only run scenarios for
        // enabled services. Disabled-service scenarios are skipped, not failed.
        let mut all = Vec::new();
        for scenario in scenarios() {
            if crate::services::service_flags::service_enabled(
                &self.app_state.db_pool,
                scenario.slug,
            )
            .await
            {
                all.push(scenario);
            } else {
                tracing::info!(
                    "⏸️ Test run {}: scenario '{}' skipped (service disabled)",
                    run_id,
                    scenario.slug
                );
            }
        }
        let total = all.len() as i32;

        let _ = sqlx::query("UPDATE test_runs SET total_tests = $1 WHERE id = $2")
            .bind(total)
            .bind(run_id)
            .execute(&self.app_state.db_pool)
            .await;

        // Resume bookkeeping. Terminal scenarios are NOT re-rendered (that is
        // the token saving); a scenario left 'running' is reattached to the
        // durable workflow that was already executing.
        let mut terminal: HashSet<String> = HashSet::new();
        let mut in_flight: HashMap<String, (Uuid, Option<Uuid>)> = HashMap::new();
        if resume {
            match sqlx::query(
                "SELECT id, gig_type, status, delivery_id FROM test_results WHERE run_id = $1",
            )
            .bind(run_id)
            .fetch_all(&self.app_state.db_pool)
            .await
            {
                Ok(rows) => {
                    for row in rows {
                        let slug: String = row.get("gig_type");
                        let status: String = row.get("status");
                        let result_id: Uuid = row.get("id");
                        let delivery_id: Option<Uuid> = row.try_get("delivery_id").ok().flatten();
                        match status.as_str() {
                            "passed" | "failed" => {
                                terminal.insert(slug);
                            }
                            "running" => {
                                in_flight.insert(slug, (result_id, delivery_id));
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => tracing::warn!("Test run {run_id}: could not load prior results ({e})"),
            }
        }

        if resume {
            let remaining = (total.max(0) as usize)
                .saturating_sub(terminal.len())
                .saturating_sub(in_flight.len());
            tracing::warn!(
                "♻️ Resuming test run {run_id}: {} already finished, {} in flight, {} to run",
                terminal.len(),
                in_flight.len(),
                remaining
            );
        }

        for scenario in &all {
            if self.run_cancelled(run_id).await {
                tracing::warn!(
                    "⏹️ Test run {} cancelled — stopping before '{}'",
                    run_id,
                    scenario.slug
                );
                break;
            }
            if !self.owns_run(run_id, token).await {
                tracing::warn!(
                    "⏹️ Test run {} — driver ownership moved, stopping before '{}'",
                    run_id,
                    scenario.slug
                );
                break;
            }
            if terminal.contains(scenario.slug) {
                tracing::info!(
                    "⏭️ Test run {}: '{}' already finished — not re-rendering",
                    run_id,
                    scenario.slug
                );
                continue;
            }
            if let Some((result_id, delivery_id)) = in_flight.get(scenario.slug).copied() {
                tracing::warn!(
                    "♻️ Test run {}: reattaching '{}' (result {}, delivery {:?})",
                    run_id,
                    scenario.slug,
                    result_id,
                    delivery_id
                );
                self.execute_and_record(run_id, scenario, result_id, delivery_id, token)
                    .await;
            } else {
                self.run_one(run_id, scenario, token).await;
            }
            // Brief pause between scenarios
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }

        // A superseded driver must not sweep or close a run that the new owner
        // is still driving.
        if !self.owns_run(run_id, token).await {
            return;
        }

        // Defensive: nothing may be left 'running' once the driver is done.
        let _ = sqlx::query(
            "UPDATE test_results SET status = 'failed', \
                 error_message = COALESCE(error_message, 'interrupted before completion'), \
                 completed_at = NOW() \
             WHERE run_id = $1 AND status = 'running'",
        )
        .bind(run_id)
        .execute(&self.app_state.db_pool)
        .await;

        self.refresh_counters(run_id).await;

        let _ = sqlx::query(
            "UPDATE test_runs \
             SET status = CASE WHEN failed_tests = 0 THEN 'completed' ELSE 'completed_with_failures' END, \
                 completed_at = NOW(), \
                 heartbeat_at = NULL \
             WHERE id = $1 AND status = 'running' AND driver_token = $2",
        )
        .bind(run_id)
        .bind(token)
        .execute(&self.app_state.db_pool)
        .await;

        tracing::info!("Portfolio test run {run_id} finished");
    }

    /// Ping `heartbeat_at` until dropped. Guarded so a panic aborts it too.
    fn start_heartbeat(&self, run_id: Uuid, token: Uuid) -> HeartbeatGuard {
        let pool = self.app_state.db_pool.clone();
        let handle = tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(RUN_HEARTBEAT_SECS));
            tick.tick().await; // immediate first tick
            loop {
                tick.tick().await;
                // Retry a dropped write instead of waiting a whole tick: Neon
                // drops the occasional connection (SSL EOF) and a missed
                // heartbeat is what makes a live run look orphaned.
                for attempt in 0..3 {
                    let res = sqlx::query(
                        "UPDATE test_runs SET heartbeat_at = NOW() \
                         WHERE id = $1 AND status = 'running' AND driver_token = $2",
                    )
                    .bind(run_id)
                    .bind(token)
                    .execute(&pool)
                    .await;
                    if res.is_ok() {
                        break;
                    }
                    if attempt == 2 {
                        tracing::warn!("Test run {run_id}: heartbeat write failed after 3 tries");
                    } else {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        });
        HeartbeatGuard(handle)
    }

    /// Claim driver ownership for this run.
    ///
    /// Returns None when another driver holds a fresh heartbeat — a duplicate
    /// spawn after a false "stale" resume. Rendering the same scenario twice
    /// burns tokens and double-counts the report.
    async fn claim_driver(&self, run_id: Uuid) -> Option<Uuid> {
        let token = Uuid::new_v4();
        sqlx::query_scalar::<_, Uuid>(
            "UPDATE test_runs SET driver_token = $2, heartbeat_at = NOW() \
              WHERE id = $1 AND status = 'running' \
                AND (driver_token IS NULL \
                     OR heartbeat_at IS NULL \
                     OR heartbeat_at < NOW() - make_interval(secs => $3)) \
             RETURNING id",
        )
        .bind(run_id)
        .bind(token)
        .bind(RUN_STALE_AFTER_SECS as f64)
        .fetch_optional(&self.app_state.db_pool)
        .await
        .ok()
        .flatten()
        .map(|_| token)
    }

    /// Does this driver still own the run? A DB hiccup answers "yes": a
    /// transient error must never make the legitimate driver abandon its work.
    async fn owns_run(&self, run_id: Uuid, token: Uuid) -> bool {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM test_runs WHERE id = $1 AND driver_token = $2)",
        )
        .bind(run_id)
        .bind(token)
        .fetch_optional(&self.app_state.db_pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(true)
    }

    /// Counters are derived from the result rows, never incremented, so a
    /// resumed run cannot double-count a scenario.
    async fn refresh_counters(&self, run_id: Uuid) {
        let _ = sqlx::query(
            "UPDATE test_runs SET \
                 passed_tests = (SELECT count(*) FROM test_results WHERE run_id = $1 AND status = 'passed'), \
                 failed_tests = (SELECT count(*) FROM test_results WHERE run_id = $1 AND status = 'failed') \
             WHERE id = $1",
        )
        .bind(run_id)
        .execute(&self.app_state.db_pool)
        .await;
    }

    /// Has an operator cancelled this run? Checked between scenarios and on every
    /// poll: without this the in-memory driver keeps advancing to the next
    /// scenario after the run row is cancelled, creating deliveries that nobody
    /// is waiting for and that then compete with the next run for workers.
    async fn run_cancelled(&self, run_id: Uuid) -> bool {
        sqlx::query_scalar::<_, bool>("SELECT status = 'cancelled' FROM test_runs WHERE id = $1")
            .bind(run_id)
            .fetch_optional(&self.app_state.db_pool)
            .await
            .ok()
            .flatten()
            .unwrap_or(false)
    }

    /// Stop the delivery and its durable workflow when the owning run is
    /// cancelled. Cancellation is cooperative for a running agent (checked at
    /// turn boundaries); a queued workflow is removed from the claim queue
    /// outright so no worker ever starts it.
    async fn cancel_delivery(&self, delivery_id: Uuid) {
        let workflow_id: Option<Uuid> = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT workflow_id FROM deliveries WHERE id = $1",
        )
        .bind(delivery_id)
        .fetch_optional(&self.app_state.db_pool)
        .await
        .ok()
        .flatten()
        .flatten();

        if let Some(wf) = workflow_id {
            let runtime = crate::services::workflow_runtime::WorkflowRuntime::new(
                self.app_state.db_pool.clone(),
            );
            let _ = runtime.request_cancel(wf).await;
            let _ = sqlx::query(
                "UPDATE app_workflows \
                    SET status = 'cancelled', completed_at = NOW(), \
                        result_summary = 'cancelled: test run was cancelled', updated_at = NOW() \
                  WHERE id = $1 AND status = 'queued'",
            )
            .bind(wf)
            .execute(&self.app_state.db_pool)
            .await;
        }

        let _ = sqlx::query(
            "UPDATE deliveries SET status = 'failed', \
                 error_message = 'RUN_CANCELLED: test run was cancelled' \
              WHERE id = $1 AND status IN ('pending', 'processing')",
        )
        .bind(delivery_id)
        .execute(&self.app_state.db_pool)
        .await;
    }

    async fn run_one(&self, run_id: Uuid, scenario: &DfyScenario, token: Uuid) {
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

        self.execute_and_record(run_id, scenario, result_id, None, token)
            .await;
    }

    /// Execute (or reattach) one scenario and record the outcome on an existing
    /// result row. `delivery_id` is Some only when resuming an interrupted run.
    async fn execute_and_record(
        &self,
        run_id: Uuid,
        scenario: &DfyScenario,
        result_id: Uuid,
        delivery_id: Option<Uuid>,
        token: Uuid,
    ) {
        let plan = match delivery_id {
            Some(id) => self.load_delivery_plan(scenario, id).await,
            None => self.prepare_delivery(scenario).await,
        };

        let plan = match plan {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("❌ '{}' could not be prepared: {e}", scenario.name);
                self.record_failure(result_id, &e).await;
                self.refresh_counters(run_id).await;
                return;
            }
        };

        // Persist the link immediately: if this task dies mid-render, the next
        // driver reattaches to this delivery instead of re-rendering it.
        let _ = sqlx::query("UPDATE test_results SET delivery_id = $1 WHERE id = $2")
            .bind(plan.delivery_id)
            .bind(result_id)
            .execute(&self.app_state.db_pool)
            .await;

        match self.drive_delivery(run_id, scenario, &plan, token).await {
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
                         llm_reviewer = $6, \
                         storage_backend = $7, \
                         completed_at = NOW() \
                     WHERE id = $8",
                )
                .bind(&r2_url)
                .bind(&r2_url)
                .bind(&filename)
                .bind(review.score)
                .bind(&review.feedback)
                .bind(&review.reviewer)
                .bind(crate::storage::backend_from_url(&r2_url).as_str())
                .bind(result_id)
                .execute(&self.app_state.db_pool)
                .await;

                tracing::info!("✅ '{}' passed — score {}/10", scenario.name, review.score);
            }
            Err(e) if e.starts_with("DRIVER_SUPERSEDED") => {
                tracing::warn!("↪️ '{}' handed over to the new driver: {e}", scenario.name);
            }
            Err(e) => {
                tracing::error!("❌ '{}' failed: {e}", scenario.name);
                self.record_failure(result_id, &e).await;
            }
        }

        self.refresh_counters(run_id).await;
    }

    async fn record_failure(&self, result_id: Uuid, err: &str) {
        let _ = sqlx::query(
            "UPDATE test_results SET \
                 status = 'failed', \
                 error_message = $1, \
                 completed_at = NOW() \
             WHERE id = $2",
        )
        .bind(err)
        .bind(result_id)
        .execute(&self.app_state.db_pool)
        .await;
    }

    /// Resolve the source and insert the delivery row. Nothing executes yet.
    async fn prepare_delivery(&self, scenario: &DfyScenario) -> Result<DeliveryPlan, String> {
        let service_def = crate::portfolio_samples::dfy_services()
            .iter()
            .find(|s| s.slug == scenario.slug)
            .ok_or_else(|| format!("Managed Campaign service '{}' not found", scenario.slug))?;

        let mut input = crate::portfolio_samples::service_input_for(service_def);
        input.source_url = crate::portfolio_samples::resolve_service_source_url(
            &self.app_state,
            service_def.slug,
            service_def.source_url,
        )
        .await;

        let delivery_id = Uuid::new_v4();
        let mut extra = crate::portfolio_samples::portfolio_extra_args(service_def);
        extra["test_run_id"] = json!(delivery_id.to_string());

        let title = format!("Test Run — {}", scenario.name);

        sqlx::query(
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

        Ok(DeliveryPlan {
            delivery_id,
            input: crate::services::agentic_service_pipeline::ServiceInput {
                title,
                brief: input.brief,
                source_url: input.source_url,
                style: input.style,
                duration_seconds: input.duration_seconds,
                delivery_id,
                prospect_id: None,
                session_uuid: None,
                user_id: None,
                source_table: Some("deliveries".to_string()),
                source_record_id: Some(delivery_id),
                idempotency_key: Some(format!("test-run:{}", scenario.slug)),
                reference_images: vec![],
            },
        })
    }

    /// Rebuild the plan for a delivery created before an interruption.
    async fn load_delivery_plan(
        &self,
        scenario: &DfyScenario,
        delivery_id: Uuid,
    ) -> Result<DeliveryPlan, String> {
        let row = sqlx::query(
            "SELECT title, prompt, style, duration, source_url FROM deliveries WHERE id = $1",
        )
        .bind(delivery_id)
        .fetch_optional(&self.app_state.db_pool)
        .await
        .map_err(|e| format!("DB read failed: {e}"))?
        .ok_or_else(|| format!("delivery {delivery_id} vanished before resume"))?;

        let title: String = row.get("title");
        Ok(DeliveryPlan {
            delivery_id,
            input: crate::services::agentic_service_pipeline::ServiceInput {
                title,
                brief: row.get("prompt"),
                source_url: row.get("source_url"),
                style: row.get("style"),
                duration_seconds: row.get("duration"),
                delivery_id,
                prospect_id: None,
                session_uuid: None,
                user_id: None,
                source_table: Some("deliveries".to_string()),
                source_record_id: Some(delivery_id),
                idempotency_key: Some(format!("test-run:{}", scenario.slug)),
                reference_images: vec![],
            },
        })
    }

    /// Start the pipeline if the delivery has no workflow yet, then poll it to
    /// completion. A resumed delivery already owns a workflow (the durable
    /// pipeline_worker keeps executing it), so we only reattach and observe.
    async fn drive_delivery(
        &self,
        run_id: Uuid,
        scenario: &DfyScenario,
        plan: &DeliveryPlan,
        token: Uuid,
    ) -> Result<(String, String), String> {
        let delivery_id = plan.delivery_id;

        let workflow_id: Option<Uuid> = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT workflow_id FROM deliveries WHERE id = $1",
        )
        .bind(delivery_id)
        .fetch_optional(&self.app_state.db_pool)
        .await
        .map_err(|e| format!("DB read failed: {e}"))?
        .flatten();

        match workflow_id {
            Some(wf) => tracing::info!("♻️ '{}': reattaching to workflow {wf}", scenario.name),
            None => {
                let started = crate::services::AgenticServicePipeline::start(
                    self.app_state.clone(),
                    scenario.slug.clone(),
                    plan.input.clone(),
                )
                .await
                .map_err(|e| format!("AgenticServicePipeline start failed: {e}"))?;

                let _ = sqlx::query("UPDATE deliveries SET workflow_id = $1 WHERE id = $2")
                    .bind(started)
                    .bind(delivery_id)
                    .execute(&self.app_state.db_pool)
                    .await;
            }
        }

        // Poll for completion. Education/landing_page runs legitimately take
        // longer than clipping. This is an *observation window* for the test
        // harness only — it no longer cancels the workflow on expiry, and the
        // agent itself has no deadlines. Configurable (default 90 min).
        let timeout_secs: u64 = std::env::var("PORTFOLIO_TEST_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5400);
        let max_polls = (timeout_secs / 5).max(1);
        for _ in 0..max_polls {
            if self.run_cancelled(run_id).await {
                self.cancel_delivery(delivery_id).await;
                return Err("RUN_CANCELLED: test run was cancelled".to_string());
            }
            if !self.owns_run(run_id, token).await {
                return Err("DRIVER_SUPERSEDED: another driver took over this run".to_string());
            }
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

        // Observation window elapsed. Do NOT cancel the workflow or fail the
        // delivery: agents are allowed to keep working past the test's wait
        // (long tasks are expected, and the pipeline is durable). Report the
        // timeout so the run result records that the harness did not observe
        // completion in time; the workflow and its delivery stay live and can
        // still finish on their own.
        Err(format!(
            "Delivery still running after {timeout_secs}s observation window — test did not observe completion (workflow left running)"
        ))
    }

    async fn review(&self, r2_url: &str, scenario: &DfyScenario) -> LLMReview {
        // Reuse the pipeline's reviewer: it extracts frames and asks a
        // multimodal model. Handing a URL to a text model only earns "cannot
        // access this video", which scored every real artifact 1/10.
        let result = crate::render_review::review_render(
            &self.app_state,
            r2_url,
            scenario.description,
            "agentic_service_video",
            None,
        )
        .await;

        LLMReview {
            score: result.score,
            feedback: result.feedback,
            reviewer: "render_review".to_string(),
        }
    }
}

// ── Orphan recovery ───────────────────────────────────────────────────────

/// Start the test-run reconciler: resume any run whose driver died.
///
/// Every replica runs this. A run is claimed with `FOR UPDATE SKIP LOCKED`, so
/// exactly one replica resumes it even when all three sweep at the same time.
/// A healthy run keeps its heartbeat fresh and is never touched.
pub fn start_reconciler(state: Arc<AppState>) {
    tokio::spawn(async move {
        reconcile_orphaned_runs(&state).await;

        let mut tick =
            tokio::time::interval(std::time::Duration::from_secs(RUN_RECONCILE_SECS));
        tick.tick().await; // skip the immediate tick — already swept above
        loop {
            tick.tick().await;
            reconcile_orphaned_runs(&state).await;
        }
    });
}

async fn reconcile_orphaned_runs(state: &Arc<AppState>) {
    let claimed = sqlx::query(
        "UPDATE test_runs SET heartbeat_at = NOW() \
         WHERE id IN ( \
             SELECT id FROM test_runs \
             WHERE status = 'running' \
               AND (heartbeat_at IS NULL \
                    OR heartbeat_at < NOW() - make_interval(secs => $1)) \
             ORDER BY started_at \
             LIMIT 3 \
             FOR UPDATE SKIP LOCKED \
         ) \
         RETURNING id",
    )
    .bind(RUN_STALE_AFTER_SECS as f64)
    .fetch_all(&state.db_pool)
    .await;

    match claimed {
        Ok(rows) => {
            for row in rows {
                let run_id: Uuid = row.get("id");
                tracing::warn!(
                    "♻️ Test run {run_id} has no live driver (heartbeat older than {RUN_STALE_AFTER_SECS}s) — resuming"
                );
                PortfolioTestRunner::spawn_run(state.clone(), run_id, true);
            }
        }
        Err(e) => tracing::warn!("Test-run reconciliation failed: {e}"),
    }
}
