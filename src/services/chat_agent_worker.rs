//! Durable claim-based runner for chat / background agents.
//!
//! Background agent runs (handlers/chat.rs) were previously dispatched with a
//! bare `tokio::spawn`: the work lived in RAM of whichever process received the
//! WebSocket message, vanished on deploy/OOM/scale events, and had unbounded
//! concurrency. Like the agentic pipeline (§37 / pipeline_worker.rs), the work
//! is now owned by a Postgres row (`agent_background_jobs`) with leases:
//!
//!   enqueue   ws_handler creates the row (status='running', claimed_by=NULL)
//!             and, when a global semaphore permit is free, claims it 'live'
//!             and runs run_agent_background() in-process.
//!   reclaim   workers here claim rows whose lease expired (owner died) or
//!             never got claimed (semaphore was full), FOR UPDATE SKIP LOCKED.
//!   headless  a reclaimed run executes run_agent_background() with no
//!             WebSocket channel; progress still persists via append_job_progress
//!             and is delivered cross-instance through job_manager/Redis.
//!   own       run_agent_background() renews the DB lease every 60s while alive.
//!   recover   supervisor requeues rows whose lease lapsed; after
//!             AGENT_CHAT_MAX_ATTEMPTS a runaway job is failed for real.
//!
//! Concurrency knob: AGENT_CHAT_WORKERS (default 2 per process).

use crate::AppState;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// The `agent_background_jobs` row returned by a successful claim.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChatClaimedJob {
    pub id: Uuid,
    pub session_uuid: String,
    pub user_message: String,
    pub workflow_id: Option<Uuid>,
    pub session_id: Option<i32>,
}

/// Identity used by in-process ('live') claims. Stable for this boot only.
fn live_instance_id() -> String {
    static INSTANCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    INSTANCE
        .get_or_init(|| {
            let suffix = std::env::var("CHAT_WORKER_INSTANCE_ID").unwrap_or_default();
            let suffix = if suffix.is_empty() {
                format!(
                    "{}-{}",
                    std::env::var("HOSTNAME").unwrap_or_else(|_| "chat-agent".to_string()),
                    std::process::id()
                )
            } else {
                suffix
            };
            format!("live-{suffix}")
        })
        .clone()
}

/// Identity used by reclaim workers (distinct from 'live' so recovery logs are
/// unambiguous).
fn reclaim_instance_id() -> &'static str {
    static INSTANCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    INSTANCE.get_or_init(|| format!("reclaim-{}", Uuid::new_v4()))
}

/// Lease TTL in minutes. The 60s renewal ticker inside run_agent_background
/// keeps it fresh; a short window means fast crash detection.
pub fn lease_minutes() -> i32 {
    std::env::var("AGENT_CHAT_LEASE_MINUTES")
        .ok()
        .and_then(|v| v.trim().parse::<i32>().ok())
        .filter(|&v| v >= 2 && v <= 720)
        .unwrap_or(30)
}

/// Background reclaim workers per process (in addition to the live in-process
/// path in the ws handler).
pub fn worker_count() -> usize {
    std::env::var("AGENT_CHAT_WORKERS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&v| v >= 1 && v <= 16)
        .unwrap_or(2)
}

fn poll_interval_secs() -> u64 {
    std::env::var("AGENT_CHAT_POLL_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| v >= 1 && v <= 300)
        .unwrap_or(5)
}

/// Safety valve: after this long the renewal ticker stops, so the supervisor
/// will eventually reclaim a pathologically hung run.
pub fn max_run_hours() -> u64 {
    std::env::var("AGENT_CHAT_MAX_HOURS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| v >= 1 && v <= 72)
        .unwrap_or(24)
}

fn max_attempts() -> i32 {
    std::env::var("AGENT_CHAT_MAX_ATTEMPTS")
        .ok()
        .and_then(|v| v.trim().parse::<i32>().ok())
        .filter(|&v| v >= 1 && v <= 10)
        .unwrap_or(3)
}

/// Claim a job row as the 'live' in-process owner. Returns true only if this
/// claim won the row (status running, not cancelled, lease unclaimed/expired).
/// Called by the ws handler BEFORE spawning its run so a concurrent supervisor
/// sweep can never double-run the job.
pub async fn claim_chat_job_live(pool: &sqlx::PgPool, job_id: Uuid) -> bool {
    let claimed_by = live_instance_id();
    match sqlx::query(
        "UPDATE agent_background_jobs
         SET claimed_by = $2,
             lease_expires_at = NOW() + make_interval(mins => $3::int),
             started_at = COALESCE(started_at, NOW()),
             updated_at = NOW()
         WHERE id = $1
           AND status = 'running'
           AND cancel_requested_at IS NULL
           AND (claimed_by IS NULL OR lease_expires_at IS NULL OR lease_expires_at < NOW())",
    )
    .bind(job_id)
    .bind(claimed_by)
    .bind(lease_minutes())
    .execute(pool)
    .await
    {
        Ok(result) => result.rows_affected() > 0,
        Err(e) => {
            tracing::warn!(%job_id, "claim_chat_job_live failed: {e}");
            false
        }
    }
}

/// Claim the oldest recoverable job for a background (reclaim) worker.
async fn claim_next_chat_job(pool: &sqlx::PgPool) -> Option<ChatClaimedJob> {
    let claimed_by = reclaim_instance_id().to_string();
    match sqlx::query_as::<_, ChatClaimedJob>(
        "UPDATE agent_background_jobs AS j
         SET claimed_by = $2,
             lease_expires_at = NOW() + make_interval(mins => $3::int),
             attempts = attempts + 1,
             started_at = COALESCE(started_at, NOW()),
             updated_at = NOW()
         FROM (
             SELECT id
             FROM agent_background_jobs
             WHERE status = 'running'
               AND cancel_requested_at IS NULL
               AND (claimed_by IS NULL OR lease_expires_at IS NULL OR lease_expires_at < NOW())
             ORDER BY created_at ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
         ) AS candidate
         WHERE j.id = candidate.id
         RETURNING j.id, j.session_uuid, j.user_message, j.workflow_id, j.session_id",
    )
    .bind(claimed_by)
    .bind(lease_minutes())
    .fetch_optional(pool)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            tracing::warn!("claim_next_chat_job failed: {e}");
            None
        }
    }
}

/// Supervisor sweep: release leases whose owner died, then fail jobs that have
/// exhausted the retry ceiling so they stop the reclaim loop.
pub async fn supervisor_sweep(pool: &sqlx::PgPool) {
    match sqlx::query(
        "UPDATE agent_background_jobs
         SET claimed_by = NULL,
             lease_expires_at = NULL,
             updated_at = NOW()
         WHERE status = 'running'
           AND claimed_by IS NOT NULL
           AND lease_expires_at IS NOT NULL
           AND lease_expires_at < NOW()",
    )
    .execute(pool)
    .await
    {
        Ok(result) => {
            if result.rows_affected() > 0 {
                tracing::warn!(
                    "🔄 chat agent supervisor released {} expired lease(s)",
                    result.rows_affected()
                );
            }
        }
        Err(e) => tracing::warn!("chat agent supervisor sweep failed: {e}"),
    }

    let ceiling = max_attempts();
    let expired_rows = sqlx::query_as::<_, (Uuid, i32)>(
        "SELECT id, attempts
         FROM agent_background_jobs
         WHERE status = 'running'
           AND claimed_by IS NULL
           AND lease_expires_at IS NULL
           AND attempts >= $1
           AND updated_at < NOW() - INTERVAL '1 minute'",
    )
    .bind(ceiling)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    for (job_id, attempts) in expired_rows {
        tracing::error!(
            %job_id,
            attempts,
            "background agent job exceeded retry ceiling; failing as runaway"
        );
        let _ = sqlx::query(
            "UPDATE agent_background_jobs
             SET status = 'failed',
                 error = $2,
                 updated_at = NOW()
             WHERE id = $1",
        )
        .bind(job_id)
        .bind(format!(
            "The background agent was reclaimed {ceiling} times without completing and was failed as a runaway/poison job."
        ))
        .execute(pool)
        .await;

        // Mark the linked workflow failed too (idempotent terminal write).
        let workflow_id = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT workflow_id FROM agent_background_jobs WHERE id = $1",
        )
        .bind(job_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten();
        if let Some(wid) = workflow_id {
            let runtime = crate::services::workflow_runtime::WorkflowRuntime::new(pool.clone());
            let _ = runtime
                .mark_failed(
                    wid,
                    Some("background_agent"),
                    "Background agent job exceeded the retry ceiling and was failed by the supervisor.",
                    None,
                )
                .await;
        }
    }
}

/// Execute a reclaimed job headless. The session owner is resolved so skill
/// attribution and user-scoped features keep working without a WebSocket.
async fn run_reclaimed_job(state: Arc<AppState>, job: ChatClaimedJob) {
    // Resolve the session owner so skill-correction attribution stays correct.
    let user_id = if let Some(sid) = job.session_id {
        sqlx::query_scalar::<_, Option<i32>>("SELECT user_id FROM chat_sessions WHERE id = $1")
            .bind(sid)
            .fetch_optional(&state.db_pool)
            .await
            .ok()
            .flatten()
            .flatten()
    } else {
        None
    };

    tracing::info!(
        "🧠 chat agent reclaim worker claimed job {} (session {})",
        job.id,
        job.session_uuid
    );

    // Headless: progress still lands in agent_background_jobs via
    // append_job_progress and reaches the user's browser on reconnect through
    // job_manager / Redis pub-sub, even with no live WebSocket channel.
    crate::handlers::chat::run_agent_background(
        state.clone(),
        job.session_uuid.clone(),
        job.user_message.clone(),
        job.user_message,
        false, // Qwen/DashScope chain, not Claude (stateful_agent.rs fallback order)
        Some(job.id),
        state.job_manager.clone(),
        None,
        None,
        user_id,
    )
    .await;
}

async fn worker_loop(state: Arc<AppState>) {
    let poll = poll_interval_secs();
    loop {
        match claim_next_chat_job(&state.db_pool).await {
            Some(job) => run_reclaimed_job(state.clone(), job).await,
            None => tokio::time::sleep(Duration::from_secs(poll)).await,
        }
    }
}

/// Spawn the reclaim worker pool + supervisor. Called once from main.rs.
pub fn start_chat_agent_workers(state: Arc<AppState>) {
    let n = worker_count();
    for i in 0..n {
        let worker_state = state.clone();
        tokio::spawn(async move {
            tracing::info!(
                "🧠 chat agent reclaim worker {i}/{n} started (instance={})",
                reclaim_instance_id()
            );
            worker_loop(worker_state).await;
        });
    }

    let sup_state = state.clone();
    tokio::spawn(async move {
        tracing::info!("🛡️ chat agent supervisor started (60s sweep)");
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await; // skip immediate first tick
        loop {
            interval.tick().await;
            supervisor_sweep(&sup_state.db_pool).await;
        }
    });
}