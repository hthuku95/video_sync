//! Durable claim-based runner for agentic service pipelines.
//!
//! Replaces the previous fire-and-forget `tokio::spawn` model where an entire
//! multi-hour render lived inside an unsupervised task on whichever Fargate
//! task received the request — work that evaporated on deploys/OOM/scaling
//! events and could not be bounded, observed, or cancelled.
//!
//! Model:
//!   enqueue   AgenticServicePipeline::start() inserts `status='queued'`.
//!   claim     N workers per process poll `claim_next_agentic_workflow()`,
//!             which uses FOR UPDATE SKIP LOCKED so concurrent Fargate tasks
//!             never double-claim. Concurrency = WORKERS × task count.
//!   own       While running, a ticker renews the DB lease every minute —
//!             this proves the owning process is alive even through long
//!             silent phases (Blender renders, LLM turns).
//!   recover   A supervisor sweep requeues rows whose lease expired, i.e.
//!             whose owner died. Reclaimed runs auto-resume from
//!             app_workflows.agent_checkpoint (<6h old).
//!   cancel    POST /api/workflows/:id/cancel sets a flag checked between
//!             agent turns; queued rows are skipped within one poll cycle.
//!
//! Global concurrency knob: AGENTIC_RENDER_WORKERS (default 1 per task;
//! 2 Fargate tasks ⇒ 2 concurrent renders ≈ the 2× g4dn GPU pool).

use crate::services::agentic_service_pipeline::AgenticServicePipeline;
use crate::services::workflow_runtime::{ClaimedWorkflow, WorkflowRuntime};
use crate::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Per-process identity used as `claimed_by`. Stable for this boot only.
fn instance_id() -> &'static str {
    static INSTANCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    INSTANCE.get_or_init(|| format!("fargate-{}", Uuid::new_v4()))
}

fn worker_count() -> usize {
    std::env::var("AGENTIC_RENDER_WORKERS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&v| v >= 1 && v <= 16)
        .unwrap_or(1)
}

fn lease_minutes() -> i32 {
    // Short lease: the 60s renewal ticker keeps it fresh; a short window means
    // fast crash detection. Must exceed one renewal interval comfortably.
    std::env::var("AGENTIC_LEASE_MINUTES")
        .ok()
        .and_then(|v| v.trim().parse::<i32>().ok())
        .filter(|&v| v >= 2 && v <= 120)
        .unwrap_or(10)
}

fn poll_interval_secs() -> u64 {
    std::env::var("AGENTIC_WORKER_POLL_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| v >= 1 && v <= 300)
        .unwrap_or(5)
}

fn max_run_hours() -> u64 {
    // Safety valve: after this long the ticker stops renewing, so the
    // supervisor will eventually reclaim a pathologically hung run.
    std::env::var("AGENTIC_MAX_RUN_HOURS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| v >= 1 && v <= 48)
        .unwrap_or(6)
}

/// Spawn the worker pool + supervisor. Called once from main.rs.
pub fn start_pipeline_infrastructure(state: Arc<AppState>) {
    let n = worker_count();
    for i in 0..n {
        let worker_state = state.clone();
        tokio::spawn(async move {
            tracing::info!(
                "🏗️ agentic pipeline worker {i}/{} started (instance={})",
                n,
                instance_id()
            );
            worker_loop(worker_state, i).await;
        });
    }

    let sup_state = state.clone();
    tokio::spawn(async move {
        tracing::info!("🛡️ agentic pipeline supervisor started (60s sweep)");
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await; // skip immediate first tick
        loop {
            interval.tick().await;
            supervisor_sweep(&sup_state).await;
        }
    });

    // Startup recovery: anything left 'running' by a dead predecessor has no
    // live ticker renewing its lease, so it requeues naturally within
    // lease_minutes of boot. Log what we found so operators see the handoff.
    let recovery_state = state.clone();
    tokio::spawn(async move {
        let runtime = WorkflowRuntime::new(recovery_state.db_pool.clone());
        match runtime.requeue_expired_leases().await {
            Ok(ids) if !ids.is_empty() => {
                tracing::info!(
                    "🔁 startup recovery: requeued {} orphaned agentic workflow(s): {:?}",
                    ids.len(),
                    ids
                );
            }
            Ok(_) => tracing::info!("✅ startup recovery: no orphaned agentic workflows"),
            Err(e) => tracing::warn!("⚠️ startup recovery check failed: {e}"),
        }
    });
}

async fn supervisor_sweep(state: &Arc<AppState>) {
    let runtime = WorkflowRuntime::new(state.db_pool.clone());

    match runtime.requeue_expired_leases().await {
        Ok(ids) if !ids.is_empty() => {
            tracing::warn!(
                "🛡️ supervisor: requeued {} agentic workflow(s) whose lease expired (owner died): {:?}",
                ids.len(),
                ids
            );
        }
        Ok(_) => {}
        Err(e) => tracing::error!("🛡️ supervisor: requeue sweep failed: {e}"),
    }

    match runtime.queued_agentic_count().await {
        Ok(depth) if depth > 0 => {
            tracing::info!("🛡️ supervisor: queue depth = {depth} pending agentic render(s)");
        }
        Ok(_) => {}
        Err(e) => tracing::debug!("🛡️ supervisor: queue depth check failed: {e}"),
    }
}

async fn worker_loop(state: Arc<AppState>, worker_idx: usize) {
    let poll = Duration::from_secs(poll_interval_secs());
    loop {
        let runtime = WorkflowRuntime::new(state.db_pool.clone());

        let claimed = match runtime
            .claim_next_agentic_workflow(instance_id(), lease_minutes())
            .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("worker[{worker_idx}] claim failed: {e}");
                tokio::time::sleep(poll).await;
                continue;
            }
        };

        let Some(job) = claimed else {
            tokio::time::sleep(poll).await;
            continue;
        };

        tracing::info!(
            "👷 worker[{worker_idx}] claimed workflow {} ({})",
            job.id,
            job.workflow_type
        );

        execute_claimed(&state, &runtime, job, worker_idx).await;
    }
}

async fn execute_claimed(
    state: &Arc<AppState>,
    runtime: &WorkflowRuntime,
    job: ClaimedWorkflow,
    worker_idx: usize,
) {
    let workflow_id = job.id;

    // Cancelled while queued? Skip without executing.
    match runtime.is_cancel_requested(workflow_id).await {
        Ok(true) => {
            tracing::info!("🚫 workflow {} was cancelled while queued — skipping", workflow_id);
            let _ = runtime
                .mark_cancelled(workflow_id, Some("cancelled"), "Cancelled before execution")
                .await;
            return;
        }
        Ok(false) => {}
        Err(e) => tracing::warn!("cancel-flag check failed (proceeding): {e}"),
    }

    // Lease-renewal ticker: proves liveness to the supervisor regardless of
    // what the pipeline is doing (long Blender renders emit no node events).
    let stop_ticker = Arc::new(AtomicBool::new(false));
    {
        let runtime = WorkflowRuntime::new(state.db_pool.clone());
        let stop = stop_ticker.clone();
        let lease_min = lease_minutes();
        let max_run = Duration::from_secs(max_run_hours() * 3600);
        let started = std::time::Instant::now();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            tick.tick().await; // immediate first renewal not needed (claim sets it)
            loop {
                tick.tick().await;
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if started.elapsed() > max_run {
                    tracing::warn!(
                        "⏰ workflow {} exceeded max run duration — stopping lease renewals",
                        workflow_id
                    );
                    break;
                }
                match runtime.renew_lease(workflow_id, instance_id(), lease_min).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!(
                            "⚠️ workflow {} lease lost to another owner — aborting renewal",
                            workflow_id
                        );
                        break;
                    }
                    Err(e) => tracing::warn!("lease renewal failed (will retry): {e}"),
                }
            }
        });
    }

    // Run the pipeline. Ownership loss mid-run surfaces as an Err from deep
    // in the agent loops (cancellation/lease probes) or as an ignored terminal
    // write here; either way we never fight a newer attempt for state.
    let result =
        AgenticServicePipeline::execute_claimed(state.clone(), &job, instance_id()).await;

    stop_ticker.store(true, Ordering::Relaxed);

    match result {
        Ok(()) => {
            tracing::info!(
                "✅ worker[{worker_idx}] workflow {} ({}) completed",
                workflow_id,
                job.workflow_type
            );
        }
        Err(err) => {
            if err.starts_with("WORKFLOW_CANCELLED:") {
                tracing::info!("🚫 workflow {workflow_id} cancelled mid-run");
                let _ = runtime
                    .mark_cancelled(workflow_id, Some("cancelled_by_request"), &err)
                    .await;
            } else if err.starts_with("WORKFLOW_LEASE_LOST") {
                tracing::warn!("🔀 workflow {workflow_id} ownership lost — another instance took over; discarding local result");
            } else {
                tracing::error!(
                    "❌ worker[{worker_idx}] workflow {} failed: {}",
                    workflow_id,
                    err
                );
                // Conditional terminal write: no-op if the supervisor already
                // requeued this workflow and another instance claimed it.
                match runtime
                    .mark_failed_if_owned(
                        workflow_id,
                        instance_id(),
                        Some("pipeline_error"),
                        &err,
                        None,
                    )
                    .await
                {
                    Ok(true) => fail_linked_delivery(state, &job, &err).await,
                    Ok(false) => tracing::warn!(
                        "workflow {workflow_id}: failure not recorded — ownership was lost"
                    ),
                    Err(e) => tracing::error!("mark_failed_if_owned failed: {e}"),
                }
            }
        }
    }
}

/// Mark the linked delivery failed for a definitively-failed workflow.
/// Delivery id lives in workflow metadata (stamped at enqueue time).
async fn fail_linked_delivery(state: &Arc<AppState>, job: &ClaimedWorkflow, err: &str) {
    let delivery_id = job
        .metadata
        .get("delivery_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .or_else(|| {
            job.source_record_id // deliveries are the usual source record
        });

    let Some(delivery_id) = delivery_id else {
        return;
    };
    let _ = sqlx::query(
        "UPDATE deliveries SET status='failed', error_message=$1 WHERE id=$2 \
         AND status NOT IN ('completed')",
    )
    .bind(err)
    .bind(delivery_id)
    .execute(&state.db_pool)
    .await;
}
