// Stateful Gemini-based clipping agent — deterministic 5-phase pipeline with PostgreSQL checkpointing.
//
// Architecture:
//   - ClippingAgentState: business-logic struct checkpointed to PostgreSQL after every phase.
//   - ClippingAgentCheckpointer: save/load/delete checkpoints (crash → resume from last phase).
//   - GeminiClippingAgent: deterministic pipeline — only ONE Gemini call per job (Phase A analysis).
//
// Gemini ONLY — no Claude references. Model split: video editing = Claude+Gemini, clipping = Gemini.
//
// Tool implementations wrap the same pub helper fns as execute_clipping_job() in clipping_job.rs.
// execute_clipping_job() is kept intact as fallback + for tests.

use crate::AppState;
use crate::clipping::{
    ai_clipper::AiClipper,
    apify_client::ApifyClient,
    gemini_video_analyzer::VideoAnalysis,
    uploader::ClipUploader,
};
use crate::gemini_client::Content;
use crate::jobs::{JobStatus, ProgressUpdate};
use crate::jobs::clipping_job::{
    count_clips_posted_today, fetch_destination_channel, fetch_job_details, fetch_linkage,
    load_clips_from_db, mark_job_completed, save_clips_to_database, update_job_status,
    update_linkage_session_timestamp, update_linkage_stats,
};
use crate::services::VideoVectorizationService;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// ClippingAgentState — checkpointed business-logic struct
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ClippingAgentState {
    pub job_id: i32,
    pub checkpoint_num: usize,

    // Phase completion flags — idempotency: completed phases are never re-executed
    pub phase_a_complete: bool,
    pub phase_b_complete: bool,
    pub phase_c_complete: bool,
    pub phase_d_complete: bool,
    pub phase_e_complete: bool,

    // Phase outputs cached so Gemini can reason about them without extra DB lookups
    pub overall_quality: Option<f64>,
    pub local_video_path: Option<String>,
    pub extracted_clip_ids: Vec<i32>,
    pub clips_uploaded: i32,
    pub clips_total: i32,

    // Terminal state
    pub terminal_status: Option<String>, // "completed", "no_clips_found", "failed"
    pub error_message: Option<String>,

    // Twitch fallback
    /// True once we have switched from YouTube to a Twitch VOD.
    pub twitch_fallback_triggered: bool,
    /// When set, this URL overrides the YouTube URL for Phase A and Phase B.
    pub active_video_url: Option<String>,

    // Metadata
    pub step_count: usize,
    pub started_at: DateTime<Utc>,
    pub last_checkpoint_at: DateTime<Utc>,
}

impl ClippingAgentState {
    fn new(job_id: i32) -> Self {
        let now = Utc::now();
        Self {
            job_id,
            checkpoint_num: 0,
            started_at: now,
            last_checkpoint_at: now,
            ..Default::default()
        }
    }

    /// Percentage of 5 phases completed (0.0–1.0).
    pub fn phases_done_pct(&self) -> f64 {
        let done = [
            self.phase_a_complete,
            self.phase_b_complete,
            self.phase_c_complete,
            self.phase_d_complete,
            self.phase_e_complete,
        ]
        .iter()
        .filter(|&&b| b)
        .count();
        done as f64 / 5.0
    }

    fn phases_done_count(&self) -> usize {
        [
            self.phase_a_complete,
            self.phase_b_complete,
            self.phase_c_complete,
            self.phase_d_complete,
            self.phase_e_complete,
        ]
        .iter()
        .filter(|&&b| b)
        .count()
    }
}

// ============================================================================
// ClippingAgentCheckpointer — PostgreSQL-backed checkpoint store
// ============================================================================

pub struct ClippingAgentCheckpointer {
    pool: sqlx::PgPool,
}

impl ClippingAgentCheckpointer {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Save state + full Gemini conversation history after each tool call.
    /// ON CONFLICT updates existing row so checkpoint_num is idempotent.
    pub async fn save(
        &self,
        state: &ClippingAgentState,
        gemini_history: &[Content],
        phase_completed: Option<&str>,
    ) -> Result<(), String> {
        let state_json =
            serde_json::to_value(state).map_err(|e| format!("Failed to serialize state: {}", e))?;
        let history_json = serde_json::to_value(gemini_history)
            .map_err(|e| format!("Failed to serialize history: {}", e))?;

        sqlx::query(
            "INSERT INTO clipping_agent_checkpoints
             (job_id, checkpoint_num, phase_completed, agent_state, gemini_history)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (job_id, checkpoint_num) DO UPDATE
               SET phase_completed = EXCLUDED.phase_completed,
                   agent_state     = EXCLUDED.agent_state,
                   gemini_history  = EXCLUDED.gemini_history",
        )
        .bind(state.job_id)
        .bind(state.checkpoint_num as i32)
        .bind(phase_completed)
        .bind(&state_json)
        .bind(&history_json)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to save checkpoint: {}", e))?;

        Ok(())
    }

    /// Load latest checkpoint for a job. Returns None if job is new.
    pub async fn load_latest(
        &self,
        job_id: i32,
    ) -> Result<Option<(ClippingAgentState, Vec<Content>)>, String> {
        let row: Option<(Value, Value)> = sqlx::query_as(
            "SELECT agent_state, gemini_history
             FROM clipping_agent_checkpoints
             WHERE job_id = $1
             ORDER BY checkpoint_num DESC
             LIMIT 1",
        )
        .bind(job_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("Failed to load checkpoint: {}", e))?;

        match row {
            None => Ok(None),
            Some((state_json, history_json)) => {
                let state: ClippingAgentState = serde_json::from_value(state_json)
                    .map_err(|e| format!("Failed to deserialize state: {}", e))?;
                let history: Vec<Content> = serde_json::from_value(history_json)
                    .map_err(|e| format!("Failed to deserialize history: {}", e))?;
                Ok(Some((state, history)))
            }
        }
    }

    /// Delete all checkpoints after successful completion (cleanup).
    pub async fn delete_all(&self, job_id: i32) -> Result<(), String> {
        sqlx::query("DELETE FROM clipping_agent_checkpoints WHERE job_id = $1")
            .bind(job_id)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("Failed to delete checkpoints: {}", e))?;
        Ok(())
    }
}

// ============================================================================
// GeminiClippingAgent — main agent
// ============================================================================

pub struct GeminiClippingAgent {
    app_state: Arc<AppState>,
    checkpointer: ClippingAgentCheckpointer,
}

impl GeminiClippingAgent {
    pub fn new(app_state: Arc<AppState>) -> Self {
        let pool = app_state.db_pool.clone();
        Self {
            checkpointer: ClippingAgentCheckpointer::new(pool),
            app_state,
        }
    }

    /// Entry point: process one clipping job via deterministic 5-phase pipeline.
    ///
    /// Uses exactly ONE Gemini API call per job (Phase A: video analysis).
    /// All other phases (download, extract, vectorize, upload) run without Gemini.
    /// Crash-safe: state is checkpointed to PostgreSQL after every phase.
    pub async fn process_job(&self, job_id: i32) -> Result<String, String> {
        // Gemini is needed for Phase A video analysis
        self.app_state
            .gemini_client
            .as_ref()
            .ok_or("Gemini client not configured — required for Phase A video analysis")?;

        // Load checkpoint (crash resumption) or create fresh state from resume_from
        let (mut state, _history) = self.load_or_init_state(job_id).await?;

        tracing::info!(
            "🤖 GeminiClippingAgent job {}: deterministic pipeline \
             (checkpoint_num={}, phase_flags=[{},{},{},{},{}])",
            job_id,
            state.checkpoint_num,
            state.phase_a_complete, state.phase_b_complete, state.phase_c_complete,
            state.phase_d_complete, state.phase_e_complete,
        );

        self.send_progress(job_id, &state, "Agent started", "starting").await;

        // ── Phase A: Video analysis (ONE Gemini call per job) ────────────────
        if !state.phase_a_complete {
            self.send_progress(job_id, &state, "analyze_video_for_clips", "running").await;
            match self.tool_analyze_video(&mut state, job_id).await {
                Ok(result) => {
                    let quality = result["overall_quality"].as_f64().unwrap_or(0.0);
                    if quality < 0.6 {
                        let reason = format!(
                            "Video quality {:.2} is below minimum threshold 0.60 — skipping download",
                            quality
                        );
                        let mut args = HashMap::new();
                        args.insert("reason".to_string(), json!(reason));
                        self.tool_mark_failed(&args, &mut state, job_id).await.ok();
                        self.checkpointer.delete_all(job_id).await.ok();
                        return Ok(format!("Job {} terminated: quality_too_low ({:.2})", job_id, quality));
                    }
                    self.advance_checkpoint(&mut state).await;
                }
                Err(e) => {
                    let mut args = HashMap::new();
                    args.insert("reason".to_string(), json!(e));
                    self.tool_mark_failed(&args, &mut state, job_id).await.ok();
                    self.checkpointer.delete_all(job_id).await.ok();
                    return Err(format!("Phase A (video analysis) failed: {}", e));
                }
            }
        }

        // ── Phase B: Download video (with Twitch fallback) ───────────────────
        if !state.phase_b_complete {
            self.send_progress(job_id, &state, "download_video", "running").await;
            match self.tool_download_video(&mut state, job_id).await {
                Ok(ref result) if result["twitch_fallback"].as_bool() == Some(true) => {
                    // Twitch fallback triggered — YouTube download failed.
                    // Gemini cannot fetch Twitch URLs directly (HTTP 400), so we download
                    // the VOD first, then analyze the local file via extracted frames.
                    //
                    // VODs can be deleted/expired between Twitch API listing and actual download.
                    // Retry up to 3 different VODs: blacklist each failed VOD so pick_twitch_vod
                    // skips it next time, then pick the next available one.
                    tracing::info!(
                        "Job {}: Twitch fallback — downloading VOD before analysis",
                        job_id
                    );

                    let mut twitch_downloaded = false;
                    let mut twitch_last_err = String::new();

                    for attempt in 0..3u32 {
                        match self.tool_download_video(&mut state, job_id).await {
                            Ok(_) => {
                                twitch_downloaded = true;
                                break;
                            }
                            Err(ref e) => {
                                twitch_last_err = e.clone();
                                tracing::warn!(
                                    "Job {}: Twitch VOD download attempt {} failed: {}",
                                    job_id, attempt + 1, e
                                );

                                // Blacklist the failed VOD so pick_twitch_vod skips it
                                if let Ok(jd) = fetch_job_details(job_id, &self.app_state.db_pool).await {
                                    if let Some(ref vid_id) = jd.twitch_video_id {
                                        self.record_clipped_twitch_video(job_id, vid_id, &jd.source_video_title).await;
                                    }
                                }

                                if attempt >= 2 {
                                    break; // 3 attempts exhausted
                                }

                                // Try to pick the next available VOD from the channel
                                match self.pick_twitch_vod(job_id).await {
                                    Some(next_vod) => {
                                        tracing::info!(
                                            "Job {}: switching to next Twitch VOD {} (attempt {})",
                                            job_id, next_vod.id, attempt + 2
                                        );
                                        sqlx::query(
                                            "UPDATE clipping_jobs \
                                             SET twitch_video_id = $1, active_video_url = $2 \
                                             WHERE id = $3",
                                        )
                                        .bind(&next_vod.id)
                                        .bind(&next_vod.url)
                                        .bind(job_id)
                                        .execute(&self.app_state.db_pool)
                                        .await
                                        .ok();
                                        state.active_video_url = Some(next_vod.url.clone());
                                    }
                                    None => {
                                        twitch_last_err = format!(
                                            "{} — no more Twitch VODs available",
                                            e
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    if !twitch_downloaded {
                        let mut args = HashMap::new();
                        args.insert("reason".to_string(), json!(twitch_last_err));
                        self.tool_mark_failed(&args, &mut state, job_id).await.ok();
                        self.checkpointer.delete_all(job_id).await.ok();
                        return Err(format!("Phase B (Twitch download) failed: {}", twitch_last_err));
                    }

                    // VOD downloaded — analyze via local frames (Gemini cannot fetch Twitch URLs)
                    match self.tool_analyze_twitch_vod(&mut state, job_id).await {
                        Ok(reresult) => {
                            let quality =
                                reresult["overall_quality"].as_f64().unwrap_or(0.0);
                            if quality < 0.6 {
                                let mut args = HashMap::new();
                                args.insert(
                                    "reason".to_string(),
                                    json!("Twitch VOD quality below threshold"),
                                );
                                self.tool_mark_failed(&args, &mut state, job_id)
                                    .await
                                    .ok();
                                self.checkpointer.delete_all(job_id).await.ok();
                                return Ok(format!(
                                    "Job {} terminated: twitch_vod_quality_too_low",
                                    job_id
                                ));
                            }
                            // Both download (B) and analysis (A) complete
                            self.advance_checkpoint(&mut state).await;
                        }
                        Err(e) => {
                            let mut args = HashMap::new();
                            args.insert(
                                "reason".to_string(),
                                json!(format!("Twitch VOD analysis failed: {}", e)),
                            );
                            self.tool_mark_failed(&args, &mut state, job_id).await.ok();
                            self.checkpointer.delete_all(job_id).await.ok();
                            return Err(format!("Phase A (Twitch fallback) failed: {}", e));
                        }
                    }
                }
                Ok(_) => { self.advance_checkpoint(&mut state).await; }
                Err(e) => {
                    let mut args = HashMap::new();
                    args.insert("reason".to_string(), json!(e));
                    self.tool_mark_failed(&args, &mut state, job_id).await.ok();
                    self.checkpointer.delete_all(job_id).await.ok();
                    return Err(format!("Phase B (download) failed: {}", e));
                }
            }
        }

        // ── Phase C: Extract clips via FFmpeg ────────────────────────────────
        if !state.phase_c_complete {
            self.send_progress(job_id, &state, "extract_clips_from_video", "running").await;
            match self.tool_extract_clips(&mut state, job_id).await {
                Ok(_) => { self.advance_checkpoint(&mut state).await; }
                Err(e) => {
                    let mut args = HashMap::new();
                    args.insert("reason".to_string(), json!(e));
                    self.tool_mark_failed(&args, &mut state, job_id).await.ok();
                    self.checkpointer.delete_all(job_id).await.ok();
                    return Err(format!("Phase C (clip extraction) failed: {}", e));
                }
            }
        }

        // ── Phase D: Vectorize (non-fatal — Qdrant outage must not block uploads) ─
        if !state.phase_d_complete {
            self.send_progress(job_id, &state, "vectorize_clips", "running").await;
            self.tool_vectorize_clips(&mut state, job_id).await.ok();
            state.phase_d_complete = true;
            self.advance_checkpoint(&mut state).await;
        }

        // ── Phase E: Upload clips to YouTube ─────────────────────────────────
        if !state.phase_e_complete {
            self.send_progress(job_id, &state, "upload_clips_to_youtube", "running").await;
            match self.tool_upload_clips(&mut state, job_id).await {
                Ok(result) => {
                    state.clips_uploaded = result["uploaded"].as_i64().unwrap_or(0) as i32;
                    state.clips_total = result["total"].as_i64().unwrap_or(0) as i32;
                    self.advance_checkpoint(&mut state).await;
                }
                Err(e) => {
                    let mut args = HashMap::new();
                    args.insert("reason".to_string(), json!(e));
                    self.tool_mark_failed(&args, &mut state, job_id).await.ok();
                    self.checkpointer.delete_all(job_id).await.ok();
                    return Err(format!("Phase E (upload) failed: {}", e));
                }
            }
        }

        // ── Mark complete ─────────────────────────────────────────────────────
        let mut complete_args = HashMap::new();
        complete_args.insert("clips_uploaded".to_string(), json!(state.clips_uploaded));
        complete_args.insert("clips_total".to_string(), json!(state.clips_total));
        self.tool_mark_complete(&complete_args, &mut state, job_id).await?;
        self.checkpointer.delete_all(job_id).await.ok();

        Ok(format!(
            "Job {} completed: {}/{} clips uploaded",
            job_id, state.clips_uploaded, state.clips_total
        ))
    }

    /// Persist state to DB after a phase completes. No conversation history in deterministic mode.
    async fn advance_checkpoint(&self, state: &mut ClippingAgentState) {
        state.checkpoint_num += 1;
        state.step_count += 1;
        state.last_checkpoint_at = Utc::now();
        self.checkpointer.save(state, &[], None).await.ok();
    }

    /// Load checkpoint from DB (crash resumption) or init fresh state from clipping_jobs.resume_from.
    async fn load_or_init_state(
        &self,
        job_id: i32,
    ) -> Result<(ClippingAgentState, Vec<Content>), String> {
        // Try to load existing checkpoint first
        if let Some((state, history)) = self.checkpointer.load_latest(job_id).await? {
            tracing::info!(
                "🔄 Resuming job {} from checkpoint {} (phase_a={}, phase_b={}, phase_c={}, phase_d={}, phase_e={})",
                job_id,
                state.checkpoint_num,
                state.phase_a_complete,
                state.phase_b_complete,
                state.phase_c_complete,
                state.phase_d_complete,
                state.phase_e_complete,
            );
            return Ok((state, history));
        }

        // Fresh start — read resume_from from clipping_jobs to pre-set phase flags
        let job = fetch_job_details(job_id, &self.app_state.db_pool).await?;
        let mut state = ClippingAgentState::new(job_id);

        // Map resume_from → completed phase flags so Gemini skips those phases
        match job.resume_from.as_deref().unwrap_or("") {
            "clips_extracted" => {
                state.phase_a_complete = true;
                state.phase_b_complete = true;
                state.phase_c_complete = true;
                tracing::info!("⏭️  Job {}: pre-setting phase A+B+C complete (resume_from=clips_extracted)", job_id);
            }
            "downloaded" => {
                state.phase_a_complete = true;
                state.phase_b_complete = true;
                tracing::info!("⏭️  Job {}: pre-setting phase A+B complete (resume_from=downloaded)", job_id);
            }
            "analyzed" => {
                state.phase_a_complete = true;
                tracing::info!("⏭️  Job {}: pre-setting phase A complete (resume_from=analyzed)", job_id);
            }
            _ => {
                tracing::info!("🆕 Job {}: fresh start (no resume_from)", job_id);
            }
        }

        // Clear resume_from so subsequent failures use fresh auto_retry_failed_jobs logic
        sqlx::query("UPDATE clipping_jobs SET resume_from = NULL WHERE id = $1")
            .bind(job_id)
            .execute(&self.app_state.db_pool)
            .await
            .ok();

        Ok((state, Vec::new()))
    }

    // =========================================================================
    // WebSocket progress helper
    // =========================================================================

    async fn send_progress(
        &self,
        job_id: i32,
        state: &ClippingAgentState,
        step: &str,
        status_str: &str,
    ) {
        let update = ProgressUpdate::new(
            job_id.to_string(),
            format!("Clipping job {}: {}", job_id, step),
            JobStatus::Running {
                current_step: step.to_string(),
                progress_percent: Some(state.phases_done_pct() * 100.0),
                steps_completed: state.phases_done_count(),
                total_steps: 5,
                completed_actions: None,
                current_action_detail: Some(status_str.to_string()),
            },
        );
        self.app_state
            .job_manager
            .send_progress(&job_id.to_string(), update)
            .await;
    }

    // =========================================================================
    // Tool implementations
    // =========================================================================

    async fn tool_analyze_video(
        &self,
        state: &mut ClippingAgentState,
        job_id: i32,
    ) -> Result<Value, String> {
        if state.phase_a_complete {
            return Ok(json!({"success": true, "skipped": true, "reason": "phase_a_complete=true"}));
        }

        let job = fetch_job_details(job_id, &self.app_state.db_pool).await?;
        let linkage = fetch_linkage(job.linkage_id, &self.app_state.db_pool).await?;

        // Use Twitch URL override if fallback was triggered, otherwise default to YouTube
        let video_url = state
            .active_video_url
            .clone()
            .or_else(|| job.active_video_url.clone())
            .unwrap_or_else(|| format!("https://youtube.com/watch?v={}", job.source_video_id));

        update_job_status(job_id, "analyzing", 10, None, &self.app_state.db_pool).await?;

        let gemini = self
            .app_state
            .gemini_client
            .as_ref()
            .ok_or("Gemini client not configured")?;

        // Emit heartbeat every 30s during Gemini analysis (up to 180s)
        let (stop_tx_a, stop_rx_a) = tokio::sync::watch::channel(false);
        let hb_db_a = self.app_state.db_pool.clone();
        let hb_job_id_a = job_id;
        let heartbeat_handle_a = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(30));
            let mut rx = stop_rx_a;
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        let _ = sqlx::query(
                            "UPDATE clipping_jobs \
                             SET worker_heartbeat_at = NOW(), updated_at = NOW() \
                             WHERE id = $1 AND status NOT IN ('completed','failed','cancelled','discarded')"
                        )
                        .bind(hb_job_id_a)
                        .execute(&hb_db_a)
                        .await;
                    }
                    _ = rx.changed() => break,
                }
            }
        });

        // Fetch top-performing viral factors from performance tracker to bias selection
        let learned_factors: Vec<String> = sqlx::query_scalar(
            "SELECT viral_factor FROM viral_factor_performance \
             WHERE total_clips >= 3 ORDER BY performance_score DESC LIMIT 5"
        )
        .fetch_all(&self.app_state.db_pool)
        .await
        .unwrap_or_default();

        if !learned_factors.is_empty() {
            tracing::info!(
                "📈 Passing {} learned high-performing factors to Gemini analysis",
                learned_factors.len()
            );
        }

        let gemini_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(180),
            gemini.analyze_video_from_url(
                &video_url,
                linkage.clips_per_video as usize,
                linkage.min_clip_duration_seconds as f64,
                linkage.max_clip_duration_seconds as f64,
                &learned_factors,
            ),
        )
        .await
        .map_err(|_| "Gemini analysis timed out after 180s".to_string())?;

        let _ = stop_tx_a.send(true);
        let _ = heartbeat_handle_a.await;

        let analysis = match gemini_result {
            Ok(a) => a,
            Err(e) if e.to_string().contains("429") || e.to_string().to_lowercase().contains("quota") => {
                tracing::warn!("⚠️ Gemini 429 on Phase A analysis (job {}), falling back to BlenderMCPServer: {}", job_id, e);
                if let Some(blender) = self.app_state.blender_mcp_client.as_ref() {
                    blender.analyze_video(
                        &video_url,
                        linkage.clips_per_video as u32,
                        linkage.min_clip_duration_seconds as f64,
                        linkage.max_clip_duration_seconds as f64,
                        &learned_factors,
                    )
                    .await
                    .map_err(|be| format!("Gemini quota exceeded (429); BlenderMCP fallback also failed: {}", be))?
                } else {
                    return Err(format!("Gemini analysis failed (429 quota): {}. No BlenderMCP fallback configured.", e));
                }
            }
            Err(e) => return Err(format!("Gemini analysis failed: {}", e)),
        };

        let overall_quality = analysis.overall_quality;
        let moments_count = analysis.viral_moments.len();
        let qualified_count = analysis.qualified_moments(0.6).len();

        // Persist full analysis to DB so retries can skip Phase A
        sqlx::query(
            "UPDATE clipping_jobs SET viral_moments_json = $1, analysis_quality = $2 WHERE id = $3",
        )
        .bind(serde_json::to_value(&analysis).unwrap_or(Value::Null))
        .bind(overall_quality)
        .bind(job_id)
        .execute(&self.app_state.db_pool)
        .await
        .ok();

        update_job_status(job_id, "analyzed", 20, None, &self.app_state.db_pool).await?;

        // Update state
        state.phase_a_complete = true;
        state.overall_quality = Some(overall_quality);

        Ok(json!({
            "success": true,
            "overall_quality": overall_quality,
            "moments_count": moments_count,
            "qualified_moments": qualified_count,
        }))
    }

    async fn tool_download_video(
        &self,
        state: &mut ClippingAgentState,
        job_id: i32,
    ) -> Result<Value, String> {
        if state.phase_b_complete {
            return Ok(json!({"success": true, "skipped": true, "reason": "phase_b_complete=true", "video_path": state.local_video_path}));
        }

        let job = fetch_job_details(job_id, &self.app_state.db_pool).await?;

        // Determine which URL to download: Twitch override or default YouTube
        let video_url = state
            .active_video_url
            .clone()
            .or_else(|| job.active_video_url.clone())
            .unwrap_or_else(|| format!("https://youtube.com/watch?v={}", job.source_video_id));

        let path = format!("downloads/clipping_{}_{}.mp4", job_id, job.source_video_id);

        update_job_status(job_id, "downloading", 25, None, &self.app_state.db_pool).await?;

        // Emit heartbeat every 30s during download (can take up to 25 min for large VODs)
        let (stop_tx_b, stop_rx_b) = tokio::sync::watch::channel(false);
        let hb_db_b = self.app_state.db_pool.clone();
        let hb_job_id_b = job_id;
        let heartbeat_handle_b = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(30));
            let mut rx = stop_rx_b;
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        let _ = sqlx::query(
                            "UPDATE clipping_jobs \
                             SET worker_heartbeat_at = NOW(), updated_at = NOW() \
                             WHERE id = $1 AND status NOT IN ('completed','failed','cancelled','discarded')"
                        )
                        .bind(hb_job_id_b)
                        .execute(&hb_db_b)
                        .await;
                    }
                    _ = rx.changed() => break,
                }
            }
        });

        // Acquire download semaphore — limits concurrent downloads to 2 to avoid
        // overwhelming the ytdlp/Apify service. Permit is held for the duration of
        // the download and released automatically when _download_permit is dropped.
        let _download_permit = self.app_state.download_semaphore.acquire().await
            .map_err(|e| format!("Download semaphore closed: {}", e))?;

        let download_result = self.download_via_apify(&video_url, &path).await;

        let _ = stop_tx_b.send(true);
        let _ = heartbeat_handle_b.await;

        match download_result {
            Ok(_) => {
                // Validate file
                if !std::path::Path::new(&path).exists() {
                    return Err(format!("Downloaded file not found: {}", path));
                }
                match crate::core::validate_video_file(&path) {
                    Ok(true) => {}
                    _ => {
                        let _ = tokio::fs::remove_file(&path).await;
                        return Err(format!("Downloaded video is corrupted: {}", path));
                    }
                }

                // If this was a Twitch fallback download, record it in clipped_twitch_videos
                if state.twitch_fallback_triggered {
                    if let Some(twitch_vid_id) = &job.twitch_video_id {
                        self.record_clipped_twitch_video(job_id, twitch_vid_id, &job.source_video_title)
                            .await;
                    }
                }

                // Persist path to DB
                sqlx::query(
                    "UPDATE clipping_jobs SET local_video_path = $1, started_at = $2 WHERE id = $3",
                )
                .bind(&path)
                .bind(Utc::now())
                .bind(job_id)
                .execute(&self.app_state.db_pool)
                .await
                .ok();

                update_job_status(job_id, "downloaded", 40, None, &self.app_state.db_pool).await?;

                state.phase_b_complete = true;
                state.local_video_path = Some(path.clone());

                Ok(json!({"success": true, "video_path": path}))
            }
            Err(download_err) => {
                // Already on Twitch fallback — give up
                if state.twitch_fallback_triggered {
                    return Err(format!(
                        "Twitch download also failed: {}",
                        download_err
                    ));
                }

                // Try Twitch fallback
                tracing::warn!(
                    "Job {}: YouTube download failed ({}), attempting Twitch fallback",
                    job_id,
                    download_err
                );

                match self.pick_twitch_vod(job_id).await {
                    Some(twitch_vod) => {
                        // Persist fallback metadata to DB
                        sqlx::query(
                            "UPDATE clipping_jobs
                             SET used_twitch_fallback = true,
                                 twitch_video_id = $1,
                                 active_video_url = $2
                             WHERE id = $3",
                        )
                        .bind(&twitch_vod.id)
                        .bind(&twitch_vod.url)
                        .bind(job_id)
                        .execute(&self.app_state.db_pool)
                        .await
                        .ok();

                        // Reset Phase A so Gemini re-analyzes the Twitch video
                        state.twitch_fallback_triggered = true;
                        state.active_video_url = Some(twitch_vod.url.clone());
                        state.phase_a_complete = false;

                        Ok(json!({
                            "success": false,
                            "twitch_fallback": true,
                            "twitch_url": twitch_vod.url,
                            "message": "YouTube download failed. Switched to Twitch VOD. \
                                        Call analyze_video_for_clips then download_video again."
                        }))
                    }
                    None => Err(format!(
                        "All YouTube download strategies failed and no Twitch mapping exists: {}",
                        download_err
                    )),
                }
            }
        }
    }

    /// Analyze a Twitch VOD that has already been downloaded to `state.local_video_path`.
    ///
    /// Gemini cannot fetch Twitch URLs directly, so after a Twitch fallback download we use
    /// `analyze_video_from_local_file` which extracts frames and sends them as images.
    /// Produces the same `VideoAnalysis` schema and persists the result to DB identically to
    /// `tool_analyze_video`.
    async fn tool_analyze_twitch_vod(
        &self,
        state: &mut ClippingAgentState,
        job_id: i32,
    ) -> Result<Value, String> {
        let video_path = state
            .local_video_path
            .clone()
            .ok_or("local_video_path not set — Twitch VOD must be downloaded before analysis")?;

        let job = fetch_job_details(job_id, &self.app_state.db_pool).await?;
        let linkage = fetch_linkage(job.linkage_id, &self.app_state.db_pool).await?;

        update_job_status(job_id, "analyzing", 10, None, &self.app_state.db_pool).await?;

        let gemini = self
            .app_state
            .gemini_client
            .as_ref()
            .ok_or("Gemini client not configured")?;

        // Fetch learned high-performing viral factors to bias analysis
        let learned_factors: Vec<String> = sqlx::query_scalar(
            "SELECT viral_factor FROM viral_factor_performance \
             WHERE total_clips >= 3 ORDER BY performance_score DESC LIMIT 5",
        )
        .fetch_all(&self.app_state.db_pool)
        .await
        .unwrap_or_default();

        tracing::info!(
            "🎬 Phase A (Twitch): analyzing local file via frames — {}",
            video_path
        );

        let analysis = tokio::time::timeout(
            tokio::time::Duration::from_secs(300), // 5 min — frame analysis can take longer
            gemini.analyze_video_from_local_file(
                &video_path,
                linkage.clips_per_video as usize,
                linkage.min_clip_duration_seconds as f64,
                linkage.max_clip_duration_seconds as f64,
                &learned_factors,
            ),
        )
        .await
        .map_err(|_| "Gemini local-file analysis timed out after 300s".to_string())?
        .map_err(|e| format!("Gemini analysis failed: {}", e))?;

        let overall_quality = analysis.overall_quality;
        let moments_count = analysis.viral_moments.len();
        let qualified_count = analysis.qualified_moments(0.6).len();

        // Persist analysis to DB (same as tool_analyze_video)
        sqlx::query(
            "UPDATE clipping_jobs SET viral_moments_json = $1, analysis_quality = $2 WHERE id = $3",
        )
        .bind(serde_json::to_value(&analysis).unwrap_or(Value::Null))
        .bind(overall_quality)
        .bind(job_id)
        .execute(&self.app_state.db_pool)
        .await
        .ok();

        update_job_status(job_id, "analyzed", 20, None, &self.app_state.db_pool).await?;

        state.phase_a_complete = true;
        state.overall_quality = Some(overall_quality);

        Ok(json!({
            "success": true,
            "overall_quality": overall_quality,
            "moments_count": moments_count,
            "qualified_moments": qualified_count,
            "source": "local_frames",
        }))
    }

    /// Call Apify (the existing download path) for any URL.
    async fn download_via_apify(&self, video_url: &str, path: &str) -> Result<(), String> {
        let apify_token =
            std::env::var("APIFY_TOKEN").map_err(|_| "APIFY_TOKEN not configured")?;
        let apify_actor = std::env::var("APIFY_YOUTUBE_CLIENT_ACTOR")
            .map_err(|_| "APIFY_YOUTUBE_CLIENT_ACTOR not configured")?;

        let apify = ApifyClient::new(apify_token, apify_actor);
        apify
            .download_video(video_url, path)
            .await
            .map(|_| ()) // discard VideoDownloadResult; file is at `path`
            .map_err(|e| format!("Download failed: {}", e))
    }

    /// Find the oldest unclipped Twitch VOD for the mapped Twitch channel of this job's
    /// YouTube source channel. Returns `None` if no mapping exists or all VODs are used.
    async fn pick_twitch_vod(&self, job_id: i32) -> Option<crate::twitch_client::TwitchVideo> {
        let twitch_client = self.app_state.twitch_client.as_ref()?;

        // Get the job to find its linkage → source channel → twitch mapping
        let job = fetch_job_details(job_id, &self.app_state.db_pool).await.ok()?;
        let linkage = fetch_linkage(job.linkage_id, &self.app_state.db_pool).await.ok()?;

        // Resolve: youtube_source_channel → youtube_twitch_channel_mappings → twitch_source_channels
        let row: Option<(i32, String)> = sqlx::query_as::<_, (i32, String)>(
            "SELECT tsc.id, tsc.broadcaster_id
             FROM youtube_twitch_channel_mappings ytm
             JOIN twitch_source_channels tsc ON tsc.id = ytm.twitch_source_channel_id
             WHERE ytm.youtube_source_channel_id = $1",
        )
        .bind(linkage.source_channel_id)
        .fetch_optional(&self.app_state.db_pool)
        .await
        .ok()?;

        let (twitch_db_id, broadcaster_id) = row?;

        // Paginate through Twitch VODs; prefer oldest unclipped (FIFO)
        let mut cursor: Option<String> = None;
        let mut all_candidates: Vec<crate::twitch_client::TwitchVideo> = Vec::new();

        for _page in 0..5 {
            match twitch_client
                .get_videos(&broadcaster_id, cursor.as_deref(), 20)
                .await
            {
                Ok((videos, next_cursor)) => {
                    if videos.is_empty() {
                        break;
                    }
                    all_candidates.extend(videos);
                    cursor = next_cursor;
                    if cursor.is_none() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!("Twitch get_videos failed for job {}: {}", job_id, e);
                    break;
                }
            }
        }

        if all_candidates.is_empty() {
            tracing::info!("No Twitch VODs found for broadcaster {}", broadcaster_id);
            return None;
        }

        // Fetch IDs of already-clipped VODs for this channel
        let used_ids: Vec<String> = sqlx::query_scalar(
            "SELECT video_id FROM clipped_twitch_videos WHERE twitch_channel_id = $1",
        )
        .bind(twitch_db_id)
        .fetch_all(&self.app_state.db_pool)
        .await
        .unwrap_or_default();

        // Filter out used VODs, sort oldest first
        let mut candidates: Vec<crate::twitch_client::TwitchVideo> = all_candidates
            .into_iter()
            .filter(|v| !used_ids.contains(&v.id))
            .collect();

        candidates.sort_by_key(|v| v.published_at);

        candidates.into_iter().next()
    }

    /// Record a Twitch VOD as clipped so it won't be reused.
    async fn record_clipped_twitch_video(
        &self,
        job_id: i32,
        twitch_video_id: &str,
        video_title: &Option<String>,
    ) {
        // Look up the twitch_channel_id via the job's linkage
        let channel_id: Option<i32> = async {
            let job = fetch_job_details(job_id, &self.app_state.db_pool).await.ok()?;
            let linkage = fetch_linkage(job.linkage_id, &self.app_state.db_pool).await.ok()?;
            let row: (i32,) = sqlx::query_as::<_, (i32,)>(
                "SELECT tsc.id FROM youtube_twitch_channel_mappings ytm
                 JOIN twitch_source_channels tsc ON tsc.id = ytm.twitch_source_channel_id
                 WHERE ytm.youtube_source_channel_id = $1",
            )
            .bind(linkage.source_channel_id)
            .fetch_optional(&self.app_state.db_pool)
            .await
            .ok()??;
            Some(row.0)
        }
        .await;

        if let Some(tcid) = channel_id {
            sqlx::query(
                "INSERT INTO clipped_twitch_videos
                     (twitch_channel_id, video_id, video_title, clipping_job_id)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT DO NOTHING",
            )
            .bind(tcid)
            .bind(twitch_video_id)
            .bind(video_title.as_deref().unwrap_or(""))
            .bind(job_id)
            .execute(&self.app_state.db_pool)
            .await
            .ok();
        }
    }

    async fn tool_extract_clips(
        &self,
        state: &mut ClippingAgentState,
        job_id: i32,
    ) -> Result<Value, String> {
        if state.phase_c_complete {
            return Ok(json!({"success": true, "skipped": true, "reason": "phase_c_complete=true", "clip_ids": state.extracted_clip_ids}));
        }

        let job = fetch_job_details(job_id, &self.app_state.db_pool).await?;
        let linkage = fetch_linkage(job.linkage_id, &self.app_state.db_pool).await?;

        let video_path = state
            .local_video_path
            .clone()
            .or(job.local_video_path.clone())
            .unwrap_or_else(|| {
                format!("downloads/clipping_{}_{}.mp4", job_id, job.source_video_id)
            });

        let analysis_value = job
            .viral_moments_json
            .ok_or("viral_moments_json not in DB — cannot extract without Phase A data")?;
        let analysis: VideoAnalysis = serde_json::from_value(analysis_value)
            .map_err(|e| format!("Failed to deserialize VideoAnalysis: {}", e))?;

        let moments: Vec<_> = analysis
            .top_moments(linkage.clips_per_video as usize)
            .into_iter()
            .cloned()
            .collect();

        update_job_status(
            job_id,
            "extracting_clips",
            50,
            None,
            &self.app_state.db_pool,
        )
        .await?;

        tokio::fs::create_dir_all("outputs")
            .await
            .map_err(|e| format!("Failed to create outputs directory: {}", e))?;

        let clipper = AiClipper::new(self.app_state.clone());
        let clips = clipper
            .extract_clips_from_moments(job_id, &video_path, &moments, &analysis.content_type)
            .await?;

        if clips.is_empty() {
            return Err("All clip extractions failed".to_string());
        }

        update_job_status(job_id, "clips_extracted", 60, None, &self.app_state.db_pool).await?;

        let clip_ids =
            save_clips_to_database(job_id, &clips, &linkage, &self.app_state.db_pool).await?;

        // Update state
        state.phase_c_complete = true;
        state.extracted_clip_ids = clip_ids.clone();

        Ok(json!({
            "success": true,
            "clips_extracted": clips.len(),
            "clip_ids": clip_ids,
        }))
    }

    async fn tool_vectorize_clips(
        &self,
        state: &mut ClippingAgentState,
        job_id: i32,
    ) -> Result<Value, String> {
        if state.phase_d_complete {
            return Ok(json!({"success": true, "skipped": true, "reason": "phase_d_complete=true"}));
        }

        let job = fetch_job_details(job_id, &self.app_state.db_pool).await?;
        let linkage = fetch_linkage(job.linkage_id, &self.app_state.db_pool).await?;
        let video_url = format!("https://youtube.com/watch?v={}", job.source_video_id);

        update_job_status(job_id, "vectorizing", 65, None, &self.app_state.db_pool).await?;

        if let Some(analysis_value) = job.viral_moments_json {
            if let Ok(analysis) = serde_json::from_value::<VideoAnalysis>(analysis_value) {
                match VideoVectorizationService::store_video_analysis_from_gemini(
                    &job.source_video_id,
                    &video_url,
                    Some(linkage.user_id),
                    None,
                    &analysis,
                    &self.app_state,
                )
                .await
                {
                    Ok(()) => tracing::info!("✅ Phase D: video_content stored in Qdrant"),
                    Err(e) => tracing::warn!("Phase D vectorization failed (non-fatal): {}", e),
                }
            }
        }

        // Update state (non-fatal: always mark complete even if Qdrant is unavailable)
        state.phase_d_complete = true;

        Ok(json!({"success": true}))
    }

    async fn tool_upload_clips(
        &self,
        state: &mut ClippingAgentState,
        job_id: i32,
    ) -> Result<Value, String> {
        if state.phase_e_complete {
            return Ok(json!({"success": true, "skipped": true, "reason": "phase_e_complete=true", "uploaded": state.clips_uploaded, "total": state.clips_total}));
        }

        let job = fetch_job_details(job_id, &self.app_state.db_pool).await?;
        let linkage = fetch_linkage(job.linkage_id, &self.app_state.db_pool).await?;

        update_job_status(job_id, "posting", 70, None, &self.app_state.db_pool).await?;

        let (clips, clip_ids) = load_clips_from_db(job_id, &self.app_state.db_pool).await?;

        if clips.is_empty() {
            state.phase_e_complete = true;
            state.clips_uploaded = 0;
            state.clips_total = 0;
            return Ok(json!({
                "success": true,
                "uploaded": 0,
                "total": 0,
                "message": "All clips already published",
            }));
        }

        let destination_channel =
            fetch_destination_channel(linkage.destination_channel_id, &self.app_state.db_pool)
                .await?;

        let youtube_client = self
            .app_state
            .youtube_client
            .as_ref()
            .ok_or("YouTube client not available")?;

        let oauth_client_id = self
            .app_state
            .google_oauth_client_id
            .as_ref()
            .ok_or("Google OAuth client ID not configured")?;

        let oauth_client_secret = self
            .app_state
            .google_oauth_client_secret
            .as_ref()
            .ok_or("Google OAuth client secret not configured")?;

        let uploader = ClipUploader::new(
            Arc::new(youtube_client.clone()),
            self.app_state.db_pool.clone(),
            oauth_client_id.clone(),
            oauth_client_secret.clone(),
        );

        let mut uploaded_count = 0i32;
        let total = clips.len() as i32;

        for (clip, clip_id) in clips.iter().zip(clip_ids.iter()) {
            let clips_today =
                count_clips_posted_today(destination_channel.id, &self.app_state.db_pool)
                    .await
                    .unwrap_or(0);
            if clips_today >= 4 {
                tracing::info!(
                    "Daily upload limit reached for channel '{}' — stopping uploads",
                    destination_channel.channel_name
                );
                break;
            }

            match uploader
                .upload_clip(clip, *clip_id, &destination_channel, linkage.requires_human_approval)
                .await
            {
                Ok(_) => {
                    uploaded_count += 1;
                    let progress = 70 + (uploaded_count * 30 / total);
                    update_job_status(job_id, "posting", progress, None, &self.app_state.db_pool)
                        .await
                        .ok();
                }
                Err(e) => {
                    tracing::error!("Failed to upload clip {}: {}", clip.clip_number, e);
                    let _ = uploader.mark_upload_failed(*clip_id, &e).await;
                }
            }
        }

        // Update state
        state.phase_e_complete = true;
        state.clips_uploaded = uploaded_count;
        state.clips_total = total;

        Ok(json!({
            "success": true,
            "uploaded": uploaded_count,
            "total": total,
        }))
    }

    async fn tool_mark_complete(
        &self,
        args: &HashMap<String, Value>,
        state: &mut ClippingAgentState,
        job_id: i32,
    ) -> Result<Value, String> {
        let clips_uploaded = args
            .get("clips_uploaded")
            .and_then(|v| v.as_i64())
            .unwrap_or(state.clips_uploaded as i64) as i32;
        let clips_total = args
            .get("clips_total")
            .and_then(|v| v.as_i64())
            .unwrap_or(state.clips_total as i64) as i32;

        let job = fetch_job_details(job_id, &self.app_state.db_pool).await?;
        let linkage = fetch_linkage(job.linkage_id, &self.app_state.db_pool).await?;

        update_job_status(job_id, "completed", 100, None, &self.app_state.db_pool).await?;
        mark_job_completed(job_id, &self.app_state.db_pool).await?;
        update_linkage_session_timestamp(linkage.id, &self.app_state.db_pool).await?;
        update_linkage_stats(
            linkage.id,
            clips_total,
            clips_uploaded,
            &self.app_state.db_pool,
        )
        .await?;

        // Mark video as successfully clipped — only at job completion, never at creation.
        // This allows the monitor to re-queue videos whose jobs previously failed.
        sqlx::query(
            "INSERT INTO clipped_source_videos
             (source_channel_id, video_id, video_title)
             VALUES ($1, $2, $3)
             ON CONFLICT (source_channel_id, video_id) DO NOTHING",
        )
        .bind(linkage.source_channel_id)
        .bind(&job.source_video_id)
        .bind(job.source_video_title.as_deref().unwrap_or(""))
        .execute(&self.app_state.db_pool)
        .await
        .ok();

        // Update channel health score on successful completion
        let _ = sqlx::query(
            "SELECT recalculate_channel_health($1)"
        )
        .bind(linkage.source_channel_id)
        .execute(&self.app_state.db_pool)
        .await;

        state.terminal_status = Some("completed".to_string());
        state.clips_uploaded = clips_uploaded;
        state.clips_total = clips_total;

        tracing::info!(
            "✅ GeminiClippingAgent job {} COMPLETED: {}/{} clips uploaded",
            job_id,
            clips_uploaded,
            clips_total
        );

        // Send completion progress
        self.send_progress(job_id, state, "completed", "done").await;

        Ok(json!({"success": true, "clips_uploaded": clips_uploaded, "clips_total": clips_total}))
    }

    async fn tool_mark_failed(
        &self,
        args: &HashMap<String, Value>,
        state: &mut ClippingAgentState,
        job_id: i32,
    ) -> Result<Value, String> {
        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");

        update_job_status(job_id, "failed", 0, Some(reason), &self.app_state.db_pool).await?;

        // Set completed_at for terminal state
        sqlx::query(
            "UPDATE clipping_jobs SET completed_at = NOW() WHERE id = $1 AND completed_at IS NULL",
        )
        .bind(job_id)
        .execute(&self.app_state.db_pool)
        .await
        .ok();

        state.terminal_status = Some("failed".to_string());
        state.error_message = Some(reason.to_string());

        // Update channel health score on failure (recalculate from job history)
        if let Ok(job) = fetch_job_details(job_id, &self.app_state.db_pool).await {
            if let Ok(linkage) = fetch_linkage(job.linkage_id, &self.app_state.db_pool).await {
                // Also record the last error on the channel health row
                let _ = sqlx::query(
                    "INSERT INTO source_channel_health (source_channel_id, last_error, last_error_at, updated_at)
                     VALUES ($1, $2, NOW(), NOW())
                     ON CONFLICT (source_channel_id) DO UPDATE
                     SET last_error = EXCLUDED.last_error,
                         last_error_at = EXCLUDED.last_error_at,
                         updated_at = EXCLUDED.updated_at"
                )
                .bind(linkage.source_channel_id)
                .bind(reason)
                .execute(&self.app_state.db_pool)
                .await;

                let _ = sqlx::query("SELECT recalculate_channel_health($1)")
                    .bind(linkage.source_channel_id)
                    .execute(&self.app_state.db_pool)
                    .await;
            }
        }

        tracing::warn!("❌ GeminiClippingAgent job {} FAILED: {}", job_id, reason);

        Ok(json!({"success": true, "status": "failed", "reason": reason}))
    }
}
