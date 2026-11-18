// src/jobs/outreach_batch_worker.rs
//! Outreach batch worker (owner directive Nov 16+18 2026).
//!
//! Processes durable `prospect_outreach_batches`. Two modes:
//!   • `full`         — sample-first, then email each prospect when its
//!                      sample completes (the original owner design).
//!   • `samples_only` — generate every missing sample and STOP at
//!                      `sample_ready`. NO email is ever sent; the owner
//!                      reviews the samples and can trigger sends later.
//!
//! Item lifecycle:
//!   pending_sample → sample_generating → (sample_ready | ready_to_send)
//!   → sending → email_sent/email_failed (full mode only)
//!
//! Concurrency/safety: per-item claims use SELECT … FOR UPDATE SKIP LOCKED,
//! so multiple instances can run the loop without double-processing. The
//! sample generation reuses `create_prospect_sample_pack_core` — the exact
//! same pipeline as the per-prospect admin button (interactive lane).

use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;

const POLL_SECS: u64 = 60;
const MAX_ATTEMPTS: i32 = 3;

pub async fn run_outreach_batch_worker(state: Arc<AppState>) {
    tracing::info!(
        "📬 outreach batch worker started (poll every {}s)",
        POLL_SECS
    );
    loop {
        match process_one_item(&state).await {
            Ok(true) => {
                // Work happened — short tick so batches drain briskly.
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
            Ok(false) => {
                tokio::time::sleep(std::time::Duration::from_secs(POLL_SECS)).await;
            }
            Err(e) => {
                tracing::warn!("outreach batch worker iteration error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(POLL_SECS)).await;
            }
        }
    }
}

/// Claim and process ONE item. Returns true if any state-changing work
/// happened (so the loop can poll faster while batches are active).
async fn process_one_item(state: &Arc<AppState>) -> Result<bool, String> {
    // 1. Claim one actionable item across all processing batches.
    let claim = sqlx::query(
        r#"WITH candidate AS (
             SELECT i.id
             FROM prospect_outreach_items i
             JOIN prospect_outreach_batches b ON b.id = i.batch_id
             WHERE b.status = 'processing'
               AND (
                 i.status = 'pending_sample'
                 OR (i.status = 'sample_generating' AND i.updated_at < NOW() - INTERVAL '10 minutes')
                 OR (i.status = 'ready_to_send' AND b.mode = 'full')
               )
             ORDER BY i.created_at ASC
             FOR UPDATE OF i SKIP LOCKED
             LIMIT 1
           )
           UPDATE prospect_outreach_items i
           SET status = CASE
                 WHEN i.status = 'ready_to_send' THEN 'sending'
                 ELSE 'sample_generating'
               END,
               updated_at = NOW()
           FROM candidate c
           WHERE i.id = c.id
           RETURNING i.id, i.batch_id, i.prospect_id, i.prospect_name, i.prospect_email,
                     i.status, i.delivery_id, i.attempts"#,
    )
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| format!("claim failed: {e}"))?;

    let Some(item) = claim else { return Ok(false) };

    let item_id: Uuid = item.get("id");
    let batch_id: Uuid = item.get("batch_id");
    let prospect_id: Uuid = item.get("prospect_id");
    let prospect_name: String = item.get("prospect_name");
    let prospect_email: String = item.get("prospect_email");
    let status_now: String = item.get("status");
    let delivery_id: Option<Uuid> = item.get("delivery_id");
    let attempts: i32 = item.get::<i32, _>("attempts");

    let batch_mode: String = sqlx::query_scalar(
        "SELECT COALESCE(mode, 'full') FROM prospect_outreach_batches WHERE id = $1",
    )
    .bind(batch_id)
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or_else(|_| "full".to_string());

    // 2. Email send (full mode ONLY — hard check; a samples-only batch must
    //    never email, even if an item ends up 'ready_to_send' via a legacy
    //    path). Owner directive Nov 18 2026.
    if status_now == "sending" {
        if batch_mode != "full" {
            // Convert to the samples-only terminal state — no email.
            sqlx::query(
                "UPDATE prospect_outreach_items SET status='sample_ready', updated_at=NOW() WHERE id=$1",
            )
            .bind(item_id)
            .execute(&state.db_pool)
            .await
            .map_err(|e| format!("item update failed: {e}"))?;
            tracing::warn!(
                "🛡️ samples-only batch {batch_id}: item {item_id} was ready_to_send — converted to sample_ready, no email sent"
            );
            maybe_complete_batch(state, batch_id).await;
            return Ok(true);
        }
        return send_email_for_item(
            state,
            item_id,
            batch_id,
            prospect_id,
            &prospect_name,
            &prospect_email,
            attempts,
        )
        .await;
    }

    // 3. Resolve the sample delivery id — create on first pickup.
    let delivery_id: Option<Uuid> = match delivery_id {
        Some(d) => Some(d),
        None => {
            // First pickup — create the sample delivery via the same core as
            // the admin button (interactive lane, admin-bypassed credits).
            match crate::handlers::prospects::create_prospect_sample_pack_core(
                state.clone(),
                prospect_id,
                None,
            )
            .await
            {
                Ok(json_resp) => {
                    let delivery = json_resp
                        .0
                        .get("delivery_id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| Uuid::parse_str(s).ok());
                    match delivery {
                        Some(d) => {
                            sqlx::query(
                                "UPDATE prospect_outreach_items SET delivery_id=$1, updated_at=NOW() WHERE id=$2",
                            )
                            .bind(d)
                            .bind(item_id)
                            .execute(&state.db_pool)
                            .await
                            .map_err(|e| format!("item update failed: {e}"))?;
                            tracing::info!(
                                "🎬 outreach sample generation started for {prospect_name} (delivery {d})"
                            );
                        }
                        None => {
                            // Core returned a payload without a new delivery id —
                            // treat the existing sample as ready.
                            mark_item(state, item_id, "sample_ready", None).await;
                            maybe_complete_batch(state, batch_id).await;
                            return Ok(true);
                        }
                    }
                    Some(delivery.unwrap_or_default())
                }
                Err((_, err)) => {
                    let msg = err.0.message.chars().take(300).collect::<String>();
                    let new_attempts = attempts + 1;
                    if new_attempts >= MAX_ATTEMPTS {
                        mark_item(state, item_id, "failed", Some(&msg)).await;
                        maybe_complete_batch(state, batch_id).await;
                    } else {
                        sqlx::query(
                            "UPDATE prospect_outreach_items SET status='pending_sample', attempts=$1, error_message=$2, updated_at=NOW() WHERE id=$3",
                        )
                        .bind(new_attempts)
                        .bind(&msg)
                        .bind(item_id)
                        .execute(&state.db_pool)
                        .await
                        .map_err(|e| format!("item update failed: {e}"))?;
                    }
                    return Ok(true);
                }
            }
        }
    };

    // 4. Check the sample delivery's status.
    let Some(delivery_id) = delivery_id else {
        return Ok(false);
    };
    let drow: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT status, output_r2_url FROM deliveries WHERE id = $1")
            .bind(delivery_id)
            .fetch_optional(&state.db_pool)
            .await
            .map_err(|e| format!("delivery load failed: {e}"))?;

    match drow {
        Some((dstatus, Some(url))) if dstatus.eq_ignore_ascii_case("completed") => {
            let final_status = if batch_mode == "samples_only" {
                "sample_ready" // STOP — no email in samples-only mode
            } else {
                "ready_to_send"
            };
            sqlx::query(
                "UPDATE prospect_outreach_items SET status=$1, sample_url=$2, updated_at=NOW() WHERE id=$3",
            )
            .bind(final_status)
            .bind(url)
            .bind(item_id)
            .execute(&state.db_pool)
            .await
            .map_err(|e| format!("item update failed: {e}"))?;
            tracing::info!("✅ outreach sample ready for {prospect_name} ({final_status})");
            maybe_complete_batch(state, batch_id).await;
            Ok(true)
        }
        Some((dstatus, _)) if dstatus.eq_ignore_ascii_case("failed") => {
            let new_attempts = attempts + 1;
            if new_attempts >= MAX_ATTEMPTS {
                mark_item(
                    state,
                    item_id,
                    "failed",
                    Some("sample generation failed after retries"),
                )
                .await;
                maybe_complete_batch(state, batch_id).await;
                Ok(true)
            } else {
                // Requeue a fresh generation attempt.
                sqlx::query(
                    "UPDATE prospect_outreach_items SET status='pending_sample', attempts=$1, delivery_id=NULL, error_message='sample render failed — retrying', updated_at=NOW() WHERE id=$2",
                )
                .bind(new_attempts)
                .bind(item_id)
                .execute(&state.db_pool)
                .await
                .map_err(|e| format!("item update failed: {e}"))?;
                Ok(true)
            }
        }
        _ => Ok(true), // still rendering — check again next tick
    }
}

async fn send_email_for_item(
    state: &Arc<AppState>,
    item_id: Uuid,
    batch_id: Uuid,
    prospect_id: Uuid,
    prospect_name: &str,
    prospect_email: &str,
    attempts: i32,
) -> Result<bool, String> {
    let script: Option<String> =
        sqlx::query_scalar("SELECT email_script FROM prospects WHERE id = $1")
            .bind(prospect_id)
            .fetch_optional(&state.db_pool)
            .await
            .map_err(|e| format!("script load failed: {e}"))?
            .flatten();
    let Some(script) = script.filter(|s| !s.trim().is_empty()) else {
        mark_item(
            state,
            item_id,
            "skipped",
            Some("no email_script on prospect"),
        )
        .await;
        maybe_complete_batch(state, batch_id).await;
        return Ok(true);
    };
    match crate::services::email_service::send_prospect_email(
        &state.db_pool,
        prospect_email,
        prospect_name,
        &script,
        prospect_id,
    )
    .await
    {
        Ok((_mid, log_id)) => {
            sqlx::query(
                "UPDATE prospect_outreach_items SET status='email_sent', email_log_id=$1, updated_at=NOW() WHERE id=$2",
            )
            .bind(log_id)
            .bind(item_id)
            .execute(&state.db_pool)
            .await
            .map_err(|e| format!("item update failed: {e}"))?;
            let _ = sqlx::query(
                "UPDATE prospect_outreach_batches SET emails_sent = emails_sent + 1 WHERE id=$1",
            )
            .bind(batch_id)
            .execute(&state.db_pool)
            .await;
            let _ = sqlx::query(
                "UPDATE prospects SET contact_status='contacted', updated_at=NOW() WHERE id=$1",
            )
            .bind(prospect_id)
            .execute(&state.db_pool)
            .await;
            tracing::info!("📬 outreach email sent for prospect {prospect_name}");
            maybe_complete_batch(state, batch_id).await;
        }
        Err(e) => {
            let new_attempts = attempts + 1;
            let next_status = if new_attempts >= MAX_ATTEMPTS {
                "email_failed"
            } else {
                "ready_to_send"
            };
            sqlx::query(
                "UPDATE prospect_outreach_items SET status=$1, attempts=$2, error_message=$3, updated_at=NOW() WHERE id=$4",
            )
            .bind(next_status)
            .bind(new_attempts)
            .bind(e)
            .bind(item_id)
            .execute(&state.db_pool)
            .await
            .map_err(|e2| format!("item update failed: {e2}"))?;
            maybe_complete_batch(state, batch_id).await;
        }
    }
    Ok(true)
}

async fn mark_item(state: &Arc<AppState>, item_id: Uuid, status: &str, msg: Option<&str>) {
    let _ = sqlx::query(
        "UPDATE prospect_outreach_items SET status=$1, error_message=$2, updated_at=NOW() WHERE id=$3",
    )
    .bind(status)
    .bind(msg)
    .bind(item_id)
    .execute(&state.db_pool)
    .await;
}

/// Mark the batch completed when no actionable items remain.
async fn maybe_complete_batch(state: &Arc<AppState>, batch_id: Uuid) {
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prospect_outreach_items WHERE batch_id=$1 AND status IN ('pending_sample','sample_generating','ready_to_send','sending')",
    )
    .bind(batch_id)
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(1);
    if pending == 0 {
        let _ = sqlx::query(
            "UPDATE prospect_outreach_batches SET status='completed', completed_at=NOW() WHERE id=$1",
        )
        .bind(batch_id)
        .execute(&state.db_pool)
        .await;
        tracing::info!("📦 outreach batch {batch_id} completed");
    }
}
