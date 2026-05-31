// Background job for clipping workflow orchestration — 5-PHASE ARCHITECTURE
//
// Phase A: Single Gemini video analysis via YouTube URL (replaces frame-by-frame pipeline)
// Phase B: Download video (only if Phase A found quality clips)
// Phase C: Parallel FFmpeg clip extraction
// Phase D: Lightweight vectorization (1 embedding into video_content collection)
// Phase E: Parallel YouTube upload + store_extracted_clip per clip
//
// Smart Resumption: Jobs that fail mid-pipeline resume from the appropriate phase rather
// than re-running from Phase A. auto_retry_failed_jobs() sets job.resume_from to indicate
// the entry point; execute_clipping_job() reads it and skips completed phases.

use crate::clipping::{
    ai_clipper::{AiClipper, ExtractedClipData},
    apify_client::ApifyClient,
    gemini_video_analyzer::VideoAnalysis,
    models::{ChannelLinkage, ClippingConfig, ClippingJob},
    uploader::ClipUploader,
};
use crate::models::youtube::ConnectedYouTubeChannel;
use crate::services::VideoVectorizationService;
use crate::AppState;
use chrono::Utc;
use serde_json::json;
use sqlx::{PgPool, Row};
use std::sync::Arc;

/// Phase determination for smart job resumption.
#[derive(Debug, PartialEq, PartialOrd)]
enum StartingPhase {
    A, // Gemini analysis (default — start from scratch)
    B, // Download only (Phase A already done; analysis stored in viral_moments_json)
    C, // Clip extraction only (Phase A + B done; video file on disk)
    E, // Upload only (Phases A–D done; clips already in extracted_clips table)
}

/// Execute clipping job workflow
pub async fn execute_clipping_job(job_id: i32, app_state: Arc<AppState>) -> Result<String, String> {
    tracing::info!("🎬 Starting clipping job {} (5-phase pipeline)", job_id);

    let job = fetch_job_details(job_id, &app_state.db_pool).await?;
    let linkage = fetch_linkage(job.linkage_id, &app_state.db_pool).await?;

    let config = ClippingConfig {
        clips_per_video: linkage.clips_per_video,
        min_clip_duration_seconds: linkage.min_clip_duration_seconds,
        max_clip_duration_seconds: linkage.max_clip_duration_seconds,
    };

    let video_url = format!("https://youtube.com/watch?v={}", job.source_video_id);
    let workflow_id = job.workflow_id;
    ensure_clipping_workflow_plan(
        &app_state,
        workflow_id,
        job_id,
        &job.source_video_id,
        &video_url,
    )
    .await;

    // =========================================================================
    // Phase Determination - smart resume via durable nodes first, legacy hint second
    // =========================================================================
    let node_resume_from =
        infer_clipping_resume_from_nodes(&app_state.db_pool, workflow_id).await?;
    let resume_hint = node_resume_from
        .as_deref()
        .or(job.resume_from.as_deref())
        .unwrap_or("");
    let starting_phase = match resume_hint {
        "analyzed" => StartingPhase::B,
        "downloaded" => StartingPhase::C,
        "clips_extracted" => StartingPhase::E,
        _ => StartingPhase::A,
    };
    if starting_phase != StartingPhase::A {
        tracing::info!(
            "⏭️  Job {}: Resuming from {:?} (resume_from='{}')",
            job_id,
            starting_phase,
            resume_hint
        );
        // Clear resume_from so subsequent failures use fresh auto_retry_failed_jobs logic
        sqlx::query("UPDATE clipping_jobs SET resume_from = NULL WHERE id = $1")
            .bind(job_id)
            .execute(&app_state.db_pool)
            .await
            .ok(); // non-fatal
    }

    // =========================================================================
    // Phase A: Single Gemini video analysis OR load from DB (resume path)
    // =========================================================================
    let (moments, analysis) = if starting_phase == StartingPhase::A {
        start_clipping_node(
            &app_state,
            workflow_id,
            "analysis",
            "gemini_video_analysis",
            json!({
                "job_id": job_id,
                "source_video_id": &job.source_video_id,
                "video_url": &video_url,
            }),
            "Analyzing source video for viral clipping moments",
        )
        .await;
        update_job_status(job_id, "analyzing", 10, None, &app_state.db_pool).await?;
        tracing::info!("🔍 Phase A: Analyzing via YouTube URL (1 Gemini call)");

        let analysis = if let Some(reused) =
            load_reusable_source_analysis(job_id, &job.source_video_id, &app_state.db_pool).await?
        {
            tracing::info!(
                "♻️ Phase A: Reusing source analysis for video {} from a sibling job",
                job.source_video_id
            );
            reused
        } else {
            let gemini_client = app_state
                .gemini_client
                .as_ref()
                .ok_or("Gemini client not configured — required for YouTube URL analysis")?;

            let gemini_result = tokio::time::timeout(
                tokio::time::Duration::from_secs(180), // 3 min for analysis
                gemini_client.analyze_video_from_url(
                    &video_url,
                    config.clips_per_video as usize,
                    config.min_clip_duration_seconds as f64,
                    config.max_clip_duration_seconds as f64,
                    &[],
                ),
            )
            .await
            .map_err(|_| "Gemini video analysis timed out after 180 seconds".to_string())?;

            match gemini_result {
                Ok(a) => a,
                Err(e) if should_fallback_from_gemini_video_analysis_error(&e.to_string()) => {
                    tracing::warn!(
                        "⚠️ Gemini provider error on video analysis, falling back to BlenderMCPServer: {}",
                        e
                    );
                    if let Some(blender) = app_state.blender_mcp_client.as_ref() {
                        blender
                            .analyze_video(
                                &video_url,
                                config.clips_per_video as u32,
                                config.min_clip_duration_seconds as f64,
                                config.max_clip_duration_seconds as f64,
                                &[],
                            )
                            .await
                            .map_err(|be| {
                                format!(
                                    "Gemini provider analysis failed; BlenderMCP fallback also failed: {}",
                                    be
                                )
                            })?
                    } else {
                        return Err(format!("Gemini video analysis failed with provider error: {}. No BlenderMCP fallback configured.", e));
                    }
                }
                Err(e) => return Err(format!("Gemini video analysis failed: {}", e)),
            }
        };

        // Fast-fail: if no moments meet quality threshold, skip download entirely
        let qualified_moments = analysis.qualified_moments(0.6);
        if qualified_moments.is_empty() {
            let status_msg = format!(
                "No viral moments found with quality >= 0.6 (overall quality: {:.2}). Video may not be suitable for clips.",
                analysis.overall_quality
            );
            update_job_status(
                job_id,
                "no_clips_found",
                100,
                Some(&status_msg),
                &app_state.db_pool,
            )
            .await?;
            complete_clipping_node(
                &app_state,
                workflow_id,
                "analysis",
                json!({
                    "overall_quality": analysis.overall_quality,
                    "qualified_count": 0,
                    "decision": "no_clips_found",
                }),
                "Analysis completed without enough qualified viral moments",
            )
            .await;
            skip_clipping_node(
                &app_state,
                workflow_id,
                "download",
                "No qualified clips were found, so source download was skipped.",
            )
            .await;
            skip_clipping_node(
                &app_state,
                workflow_id,
                "extract_clips",
                "No qualified clips were found, so extraction was skipped.",
            )
            .await;
            skip_clipping_node(
                &app_state,
                workflow_id,
                "vectorize",
                "No qualified clips were found, so vectorization was skipped.",
            )
            .await;
            skip_clipping_node(
                &app_state,
                workflow_id,
                "post_to_youtube",
                "No qualified clips were found, so YouTube upload was skipped.",
            )
            .await;
            skip_clipping_node(
                &app_state,
                workflow_id,
                "fallback_summary",
                "Source analysis succeeded; fallback summary was not needed.",
            )
            .await;
            mark_job_completed(job_id, &app_state.db_pool).await?;
            return Ok(format!(
                "No qualifying clips found (overall_quality={:.2})",
                analysis.overall_quality
            ));
        }

        // Hard-cap clip duration to config max (never exceed for Shorts compatibility).
        // Gemini sometimes returns moments slightly over the requested max.
        let shorts_max = config.max_clip_duration_seconds as f64;
        let moments: Vec<_> = analysis
            .top_moments(config.clips_per_video as usize)
            .into_iter()
            .cloned()
            .map(|mut m| {
                let dur = m.end_sec - m.start_sec;
                if dur > shorts_max {
                    tracing::warn!(
                        "Moment '{title}' is {dur:.1}s — truncating end to {max}s cap",
                        title = m.title,
                        dur = dur,
                        max = shorts_max
                    );
                    m.end_sec = m.start_sec + shorts_max;
                }
                m
            })
            .collect();

        tracing::info!(
            "✅ Phase A complete: {} viral moments identified ({} qualify with score ≥ 0.6)",
            analysis.viral_moments.len(),
            qualified_moments.len()
        );

        // Persist full VideoAnalysis to DB so retries can skip Gemini re-analysis.
        // Storing VideoAnalysis (not just Vec<ViralMoment>) so Phase D can reconstruct
        // the analysis object for vectorization on Phase B/C resume paths.
        persist_job_analysis(job_id, &analysis, &app_state.db_pool)
            .await
            .ok(); // non-fatal: failure just means retry re-runs Phase A

        store_source_analysis_vector(
            &job.source_video_id,
            &video_url,
            Some(linkage.user_id),
            None,
            &analysis,
            &app_state,
            "phase_a",
        )
        .await;

        update_job_status(job_id, "analyzed", 20, None, &app_state.db_pool).await?;
        complete_clipping_node(
            &app_state,
            workflow_id,
            "analysis",
            json!({
                "overall_quality": analysis.overall_quality,
                "moment_count": moments.len(),
                "qualified_count": qualified_moments.len(),
            }),
            "Source analysis persisted for clipping workflow",
        )
        .await;
        if should_pause_after_clipping_node() {
            pause_clipping_after_node(
                &app_state,
                job_id,
                workflow_id,
                "analysis",
                "analyzed",
                20,
                "Analysis node completed; job released for the download node.",
            )
            .await?;
            return Ok("Analysis node completed; job requeued for download".to_string());
        }
        (moments, analysis)
    } else {
        // Resume: load VideoAnalysis from DB (avoids Gemini re-call + API cost)
        tracing::info!("⏭️  Phase A skipped: loading analysis from DB");
        let analysis_value = job.viral_moments_json.clone().ok_or_else(|| {
            "viral_moments_json not in DB — cannot resume; will restart from Phase A".to_string()
        })?;
        let analysis: VideoAnalysis = serde_json::from_value(analysis_value)
            .map_err(|e| format!("Failed to deserialize VideoAnalysis from DB: {}", e))?;

        let shorts_max = config.max_clip_duration_seconds as f64;
        let moments: Vec<_> = analysis
            .top_moments(config.clips_per_video as usize)
            .into_iter()
            .cloned()
            .map(|mut m| {
                if m.end_sec - m.start_sec > shorts_max {
                    m.end_sec = m.start_sec + shorts_max;
                }
                m
            })
            .collect();

        tracing::info!(
            "✅ Loaded {} moments from DB (overall_quality={:.2})",
            moments.len(),
            analysis.overall_quality
        );
        complete_clipping_node(
            &app_state,
            workflow_id,
            "analysis",
            json!({
                "overall_quality": analysis.overall_quality,
                "moment_count": moments.len(),
                "resumed_from": &job.resume_from,
            }),
            "Reused persisted source analysis for clipping workflow",
        )
        .await;
        (moments, analysis)
    };

    // =========================================================================
    // Phase B: Download video (skipped if video already on disk)
    // =========================================================================
    let video_path = if starting_phase <= StartingPhase::B {
        let path = format!("downloads/clipping_{}_{}.mp4", job_id, job.source_video_id);

        start_clipping_node(
            &app_state,
            workflow_id,
            "download",
            "source_download",
            json!({
                "job_id": job_id,
                "video_url": &video_url,
                "path": &path,
            }),
            "Downloading source media for clipping",
        )
        .await;
        update_job_status(job_id, "downloading", 25, None, &app_state.db_pool).await?;
        tracing::info!("⬇️  Phase B: Downloading video");

        let apify_token = std::env::var("APIFY_TOKEN").map_err(|_| "APIFY_TOKEN not configured")?;
        let apify_actor = std::env::var("APIFY_YOUTUBE_CLIENT_ACTOR")
            .map_err(|_| "APIFY_YOUTUBE_CLIENT_ACTOR not configured")?;

        let apify_client = ApifyClient::new(apify_token, apify_actor);

        tracing::info!("Downloading video: {}", video_url);
        let download_timeout_secs = clipping_download_timeout_secs();
        let download_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(download_timeout_secs),
            apify_client.download_video(&video_url, &path),
        )
        .await
        .map_err(|_| {
            format!(
                "Download timed out after {}s for {}",
                download_timeout_secs, video_url
            )
        })
        .and_then(|result| result.map_err(|e| format!("Download failed: {}", e)));

        if let Err(download_error) = download_result {
            fail_clipping_node(
                &app_state,
                workflow_id,
                "download",
                &download_error,
                json!({
                    "job_id": job_id,
                    "fallback": "generated_summary_delivery",
                }),
            )
            .await;
            start_clipping_node(
                &app_state,
                workflow_id,
                "fallback_summary",
                "generated_summary_delivery",
                json!({
                    "job_id": job_id,
                    "source_video_id": &job.source_video_id,
                    "failure_reason": &download_error,
                }),
                "Queuing generated fallback summary after source download failure",
            )
            .await;
            let fallback_result = handle_download_failure_fallback(
                &job,
                &linkage,
                &video_url,
                &analysis,
                &app_state,
                &download_error,
            )
            .await;
            match &fallback_result {
                Ok(message) => {
                    complete_clipping_node(
                        &app_state,
                        workflow_id,
                        "fallback_summary",
                        json!({ "message": message }),
                        "Fallback summary delivery queued",
                    )
                    .await;
                }
                Err(error) => {
                    fail_clipping_node(
                        &app_state,
                        workflow_id,
                        "fallback_summary",
                        error,
                        json!({ "job_id": job_id }),
                    )
                    .await;
                }
            }
            return fallback_result;
        }

        // Validate downloaded file
        if !std::path::Path::new(&path).exists() {
            return Err(format!("Downloaded file not found: {}", path));
        }

        match crate::core::validate_video_file(&path) {
            Ok(true) => tracing::info!("✅ Downloaded video validated"),
            Ok(false) | Err(_) => {
                let _ = tokio::fs::remove_file(&path).await;
                return Err(format!("Downloaded video is corrupted: {}", path));
            }
        }

        update_job_status(job_id, "downloaded", 40, None, &app_state.db_pool).await?;
        update_job_video_path(job_id, &path, &app_state.db_pool).await?;
        complete_clipping_node(
            &app_state,
            workflow_id,
            "download",
            json!({ "path": &path }),
            "Source media downloaded and validated",
        )
        .await;

        // Upload raw download to R2 for persistence across restarts (best-effort)
        if let Some(r2) = app_state.r2_client.as_ref() {
            let ext = if job.used_twitch_fallback {
                "mp4"
            } else {
                "mp4"
            };
            let raw_key = crate::r2_client::R2Client::key_raw_download(
                linkage.user_id,
                &job.source_video_id,
                ext,
            );
            match r2.upload(&path, &raw_key).await {
                Ok(()) => {
                    tracing::info!("R2: uploaded raw download → {}", raw_key);
                    sqlx::query("UPDATE clipping_jobs SET r2_raw_key = $1 WHERE id = $2")
                        .bind(&raw_key)
                        .bind(job_id)
                        .execute(&app_state.db_pool)
                        .await
                        .ok();
                }
                Err(e) => tracing::warn!("R2 raw upload failed (non-fatal): {}", e),
            }
        }

        if should_pause_after_clipping_node() {
            pause_clipping_after_node(
                &app_state,
                job_id,
                workflow_id,
                "download",
                "downloaded",
                40,
                "Download node completed; job released for the extraction/vectorization node.",
            )
            .await?;
            return Ok("Download node completed; job requeued for extraction".to_string());
        }

        path
    } else {
        // Resume from Phase C or E: use the path stored in DB at download time
        let path = job.local_video_path.clone().unwrap_or_else(|| {
            format!("downloads/clipping_{}_{}.mp4", job_id, job.source_video_id)
        });
        tracing::info!("⏭️  Phase B skipped: using stored path '{}'", path);
        path
    };

    // =========================================================================
    // Phase C: Parallel clip extraction + Phase D: Lightweight vectorization
    // (Skipped entirely on Phase E resume — clips already in extracted_clips table)
    // =========================================================================
    let (clips, clip_db_ids) = if starting_phase <= StartingPhase::C {
        start_clipping_node(
            &app_state,
            workflow_id,
            "extract_clips",
            "clip_extraction",
            json!({
                "job_id": job_id,
                "video_path": &video_path,
                "moment_count": moments.len(),
            }),
            "Extracting clips from selected moments",
        )
        .await;
        update_job_status(job_id, "extracting_clips", 50, None, &app_state.db_pool).await?;
        tracing::info!(
            "✂️  Phase C: Parallel clip extraction ({} clips)",
            moments.len()
        );

        // Create output directory
        tokio::fs::create_dir_all("outputs")
            .await
            .map_err(|e| format!("Failed to create outputs directory: {}", e))?;

        let clipper = AiClipper::new(app_state.clone());
        let mut clips = clipper
            .extract_clips_from_moments(job_id, &video_path, &moments, &analysis.content_type)
            .await?;

        let min_duration = config.min_clip_duration_seconds as f64;
        let max_duration = config.max_clip_duration_seconds as f64;
        let before_duration_filter = clips.len();
        clips.retain(|clip| {
            clip.duration_seconds >= min_duration && clip.duration_seconds <= max_duration
        });

        if clips.len() != before_duration_filter {
            tracing::warn!(
                "Clipping job {} dropped {} clip(s) outside configured duration bounds (min={}s, max={}s)",
                job_id,
                before_duration_filter.saturating_sub(clips.len()),
                config.min_clip_duration_seconds,
                config.max_clip_duration_seconds
            );
        }

        if clips.is_empty() {
            return Err("All clip extractions failed".to_string());
        }

        tracing::info!(
            "✅ Phase C complete: {}/{} clips extracted",
            clips.len(),
            moments.len()
        );
        update_job_status(job_id, "clips_extracted", 60, None, &app_state.db_pool).await?;
        complete_clipping_node(
            &app_state,
            workflow_id,
            "extract_clips",
            json!({ "clip_count": clips.len() }),
            "Clips extracted from source media",
        )
        .await;

        enhance_clips_with_full_agent(
            job_id,
            linkage.user_id,
            &video_url,
            &analysis.content_type,
            &mut clips,
            &app_state,
            "auto_clipping",
        )
        .await;

        // Phase C→D: Upload clips + thumbnails to R2 (best-effort, non-fatal)
        let clips = if let Some(r2) = app_state.r2_client.as_ref() {
            let mut clips_with_r2 = clips;
            for (i, clip) in clips_with_r2.iter_mut().enumerate() {
                let clip_n = (i + 1) as usize;
                let clip_key = crate::r2_client::R2Client::key_clip(job_id, clip_n);
                match r2.upload(&clip.local_clip_path, &clip_key).await {
                    Ok(()) => match r2.presign_get(&clip_key, 86400).await {
                        Ok(url) => {
                            clip.r2_clip_key = Some(clip_key.clone());
                            clip.r2_clip_url = Some(url);
                            tracing::info!("R2: uploaded clip {} → {}", clip_n, clip_key);
                        }
                        Err(e) => tracing::warn!("R2 presign failed for clip {}: {}", clip_n, e),
                    },
                    Err(e) => tracing::warn!("R2 upload failed for clip {}: {}", clip_n, e),
                }
                if let Some(thumb_path) = &clip.custom_thumbnail_path {
                    let thumb_key = crate::r2_client::R2Client::key_thumbnail(job_id, clip_n);
                    match r2.upload(thumb_path, &thumb_key).await {
                        Ok(()) => {
                            clip.r2_thumb_key = Some(thumb_key.clone());
                            tracing::info!("R2: uploaded thumbnail {} → {}", clip_n, thumb_key);
                        }
                        Err(e) => {
                            tracing::warn!("R2 upload failed for thumbnail {}: {}", clip_n, e)
                        }
                    }
                }
            }
            clips_with_r2
        } else {
            clips
        };

        // Phase D: Lightweight vectorization (1 embedding into video_content)
        start_clipping_node(
            &app_state,
            workflow_id,
            "vectorize",
            "source_vectorization",
            json!({
                "job_id": job_id,
                "source_video_id": &job.source_video_id,
                "model_lane": "gemini_multimodal_or_configured_embedding",
            }),
            "Persisting source analysis vectors for learning and reuse",
        )
        .await;
        update_job_status(job_id, "vectorizing", 65, None, &app_state.db_pool).await?;
        tracing::info!("🔢 Phase D: Storing video analysis (1 embedding)");

        store_source_analysis_vector(
            &job.source_video_id,
            &video_url,
            Some(linkage.user_id),
            None, // channel_id set from linkage if needed
            &analysis,
            &app_state,
            "phase_d",
        )
        .await;

        // Save clips to database (idempotent: deletes stale clips first)
        let clip_db_ids =
            save_clips_to_database(job_id, &clips, &linkage, &app_state.db_pool).await?;
        complete_clipping_node(
            &app_state,
            workflow_id,
            "vectorize",
            json!({
                "clip_count": clips.len(),
                "clip_db_ids": &clip_db_ids,
            }),
            "Source vectors and clip records persisted",
        )
        .await;
        if should_pause_after_clipping_node() {
            pause_clipping_after_node(
                &app_state,
                job_id,
                workflow_id,
                "vectorize",
                "clips_extracted",
                65,
                "Extraction/vectorization nodes completed; job released for the upload node.",
            )
            .await?;
            return Ok(
                "Extraction/vectorization node completed; job requeued for upload".to_string(),
            );
        }
        (clips, clip_db_ids)
    } else {
        // Phase E resume: clips already extracted and saved — load from DB
        tracing::info!("⏭️  Phases C/D skipped: loading existing clips from DB for upload retry");
        let (db_clips, db_ids) = load_clips_from_db(job_id, &app_state.db_pool).await?;

        if db_clips.is_empty() {
            // All clips already published on a previous partial run — job is done
            tracing::info!(
                "Job {}: all clips already published, marking complete",
                job_id
            );
            update_job_status(job_id, "completed", 100, None, &app_state.db_pool).await?;
            mark_job_completed(job_id, &app_state.db_pool).await?;
            complete_clipping_node(
                &app_state,
                workflow_id,
                "post_to_youtube",
                json!({ "already_published": true }),
                "All saved clips were already published",
            )
            .await;
            skip_clipping_node(
                &app_state,
                workflow_id,
                "fallback_summary",
                "Clips were already published; fallback summary was not needed.",
            )
            .await;
            return Ok("All clips already published (Phase E resume)".to_string());
        }

        tracing::info!(
            "Loaded {} unpublished clips from DB for retry",
            db_clips.len()
        );
        (db_clips, db_ids)
    };

    // =========================================================================
    // Phase E: Parallel YouTube upload
    // =========================================================================
    start_clipping_node(
        &app_state,
        workflow_id,
        "post_to_youtube",
        "youtube_upload",
        json!({
            "job_id": job_id,
            "clip_count": clips.len(),
        }),
        "Uploading generated clips to destination YouTube channel",
    )
    .await;
    update_job_status(job_id, "posting", 70, None, &app_state.db_pool).await?;
    tracing::info!("📤 Phase E: Uploading {} clips to YouTube", clips.len());

    let destination_channel =
        fetch_destination_channel(linkage.destination_channel_id, &app_state.db_pool).await?;

    let youtube_client = app_state
        .youtube_client
        .as_ref()
        .ok_or("YouTube client not available")?;

    let oauth_client_id = app_state
        .google_oauth_client_id
        .as_ref()
        .ok_or("Google OAuth client ID not configured")?;

    let oauth_client_secret = app_state
        .google_oauth_client_secret
        .as_ref()
        .ok_or("Google OAuth client secret not configured")?;

    let uploader = ClipUploader::new(
        Arc::new(youtube_client.clone()),
        app_state.db_pool.clone(),
        oauth_client_id.clone(),
        oauth_client_secret.clone(),
    );

    let mut uploaded_count = 0;
    let min_duration = config.min_clip_duration_seconds as f64;
    let max_duration = config.max_clip_duration_seconds as f64;
    for (clip, clip_id) in clips.iter().zip(clip_db_ids.iter()) {
        if clip.duration_seconds < min_duration || clip.duration_seconds > max_duration {
            tracing::warn!(
                "Skipping clip {} for job {} because duration {:.2}s is outside configured bounds (min={}s, max={}s)",
                clip.clip_number,
                job_id,
                clip.duration_seconds,
                config.min_clip_duration_seconds,
                config.max_clip_duration_seconds
            );
            let _ = uploader
                .mark_upload_failed(
                    *clip_id,
                    &format!(
                        "Skipped before upload: duration {:.2}s outside configured bounds (min={}s, max={}s)",
                        clip.duration_seconds,
                        config.min_clip_duration_seconds,
                        config.max_clip_duration_seconds
                    ),
                )
                .await;
            continue;
        }

        // Enforce daily 4-clip limit at upload time (not just at job-creation time).
        let clips_today = count_clips_posted_today(destination_channel.id, &app_state.db_pool)
            .await
            .unwrap_or(0);
        if clips_today >= 4 {
            tracing::info!(
                "Daily upload limit (4 clips) reached for channel '{}' — stopping uploads for job {}",
                destination_channel.channel_name, job_id
            );
            break;
        }

        match uploader
            .upload_clip(
                clip,
                *clip_id,
                &destination_channel,
                linkage.requires_human_approval,
            )
            .await
        {
            Ok(_) => {
                uploaded_count += 1;
                let progress = 70 + (uploaded_count * 30 / clips.len() as i32);
                update_job_status(job_id, "posting", progress, None, &app_state.db_pool).await?;

                // Store clip in extracted_clips Qdrant collection (best-effort)
                store_clip_in_qdrant(*clip_id, job_id, &job.source_video_id, clip, &app_state)
                    .await;
            }
            Err(e) => {
                tracing::error!("Failed to upload clip {}: {}", clip.clip_number, e);
                let _ = uploader.mark_upload_failed(*clip_id, &e).await;
            }
        }
    }

    // If ALL uploads failed, mark job as failed so auto-retry can resume from Phase E.
    // The auto_retry_failed_jobs() function will set resume_from='clips_extracted' so the
    // next attempt skips Phases A-D entirely and goes straight to upload.
    if uploaded_count == 0 {
        update_job_status(
            job_id,
            "failed",
            0,
            Some("All clip uploads failed. Job will retry when OAuth tokens are valid."),
            &app_state.db_pool,
        )
        .await?;
        update_linkage_stats(linkage.id, clips.len() as i32, 0, &app_state.db_pool).await?;
        let _ = tokio::fs::remove_file(&video_path).await;
        fail_clipping_node(
            &app_state,
            workflow_id,
            "post_to_youtube",
            "All clip uploads failed — check OAuth token validity",
            json!({ "job_id": job_id, "clip_count": clips.len() }),
        )
        .await;
        tracing::error!(
            "❌ Clipping job {} marked failed: 0/{} clips uploaded",
            job_id,
            clips.len()
        );
        return Err("All clip uploads failed — check OAuth token validity".to_string());
    }

    // At least 1 clip uploaded successfully — mark job completed.
    update_job_status(job_id, "completed", 100, None, &app_state.db_pool).await?;
    mark_job_completed(job_id, &app_state.db_pool).await?;
    update_linkage_session_timestamp(linkage.id, &app_state.db_pool).await?;
    update_linkage_stats(
        linkage.id,
        clips.len() as i32,
        uploaded_count,
        &app_state.db_pool,
    )
    .await?;
    complete_clipping_node(
        &app_state,
        workflow_id,
        "post_to_youtube",
        json!({
            "uploaded_count": uploaded_count,
            "clip_count": clips.len(),
        }),
        "Uploaded generated clips to YouTube",
    )
    .await;
    skip_clipping_node(
        &app_state,
        workflow_id,
        "fallback_summary",
        "Source download and clip upload succeeded; fallback summary was not needed.",
    )
    .await;

    // Mark video as successfully clipped — only written here (job completion), never at job creation.
    // This ensures the monitor can re-queue a video whose job previously failed or was cancelled.
    sqlx::query(
        "INSERT INTO clipped_source_videos
         (source_channel_id, video_id, video_title)
         VALUES ($1, $2, $3)
         ON CONFLICT (source_channel_id, video_id) DO NOTHING",
    )
    .bind(linkage.source_channel_id)
    .bind(&job.source_video_id)
    .bind(job.source_video_title.as_deref().unwrap_or(""))
    .execute(&app_state.db_pool)
    .await
    .ok(); // non-fatal

    // Cleanup downloaded video (clips in outputs/ are kept for potential re-processing)
    let _ = tokio::fs::remove_file(&video_path).await;

    tracing::info!(
        "✅ Clipping job {} completed: {}/{} clips posted",
        job_id,
        uploaded_count,
        clips.len()
    );

    Ok(format!(
        "Successfully posted {}/{} clips",
        uploaded_count,
        clips.len()
    ))
}

pub(crate) fn should_fallback_from_gemini_video_analysis_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    error.contains("429")
        || error.contains("403")
        || error.contains("PERMISSION_DENIED")
        || error.contains("RESOURCE_EXHAUSTED")
        || lower.contains("quota")
        || lower.contains("rate limit")
        || lower.contains("permission denied")
        || lower.contains("model")
}

/// Load unpublished clips from the extracted_clips table for Phase E resume.
///
/// Returns only clips with upload_status != 'published' so we retry failed/pending
/// clips without re-uploading clips that already succeeded on a previous partial run.
/// Returns (clips, clip_db_ids) — empty vecs if all clips are already published.
pub async fn load_clips_from_db(
    job_id: i32,
    pool: &PgPool,
) -> Result<(Vec<ExtractedClipData>, Vec<i32>), String> {
    let rows = sqlx::query(
        "SELECT id, clip_number, local_clip_path,
                start_time_seconds, end_time_seconds, duration_seconds,
                ai_title, ai_description, ai_tags, ai_confidence_score,
                viral_factors, custom_thumbnail_path, thumbnail_generation_method,
                enhancement_applied, enhancement_tools, enhancement_reasoning,
                r2_clip_key, r2_thumb_key, r2_clip_url,
                qa_status, qa_score, qa_feedback, qa_retry_hint
         FROM extracted_clips
         WHERE clipping_job_id = $1
           AND upload_status != 'published'
         ORDER BY clip_number",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to load clips from DB for Phase E resume: {}", e))?;

    let mut clips = Vec::new();
    let mut ids = Vec::new();

    for row in rows {
        let id: i32 = row.get("id");
        let ai_title: Option<String> = row.try_get("ai_title").ok().flatten();
        let ai_description: Option<String> = row.try_get("ai_description").ok().flatten();
        let ai_tags: Option<Vec<String>> = row.try_get("ai_tags").ok().flatten();
        let ai_confidence_score: Option<f64> = row.try_get("ai_confidence_score").ok().flatten();
        let viral_factors: Option<Vec<String>> = row.try_get("viral_factors").ok().flatten();
        let custom_thumbnail_path: Option<String> =
            row.try_get("custom_thumbnail_path").ok().flatten();
        let thumbnail_generation_method: Option<String> =
            row.try_get("thumbnail_generation_method").ok().flatten();
        let enhancement_applied: bool = row.try_get("enhancement_applied").ok().unwrap_or(false);
        let enhancement_tools: Vec<String> =
            row.try_get("enhancement_tools").ok().unwrap_or_default();
        let enhancement_reasoning: Option<String> =
            row.try_get("enhancement_reasoning").ok().flatten();

        clips.push(ExtractedClipData {
            clip_number: row.get("clip_number"),
            local_clip_path: row.get("local_clip_path"),
            start_time_seconds: row.get("start_time_seconds"),
            end_time_seconds: row.get("end_time_seconds"),
            duration_seconds: row.get("duration_seconds"),
            ai_title: ai_title.unwrap_or_else(|| "Untitled".to_string()),
            ai_description: ai_description.unwrap_or_default(),
            ai_tags: ai_tags.unwrap_or_default(),
            ai_confidence_score: ai_confidence_score.unwrap_or(0.0),
            viral_factors: viral_factors.unwrap_or_default(),
            custom_thumbnail_path,
            thumbnail_generation_method,
            enhancement_applied,
            enhancement_tools,
            enhancement_reasoning,
            r2_clip_key: row.try_get("r2_clip_key").ok().flatten(),
            r2_thumb_key: row.try_get("r2_thumb_key").ok().flatten(),
            r2_clip_url: row.try_get("r2_clip_url").ok().flatten(),
            qa_status: row.try_get("qa_status").ok().flatten(),
            qa_score: row.try_get("qa_score").ok().flatten(),
            qa_feedback: row.try_get("qa_feedback").ok().flatten(),
            qa_retry_hint: row.try_get("qa_retry_hint").ok().flatten(),
        });
        ids.push(id);
    }

    Ok((clips, ids))
}

/// Store an extracted clip in the Qdrant extracted_clips collection (best-effort, non-fatal).
async fn store_clip_in_qdrant(
    clip_id: i32,
    job_id: i32,
    source_video_id: &str,
    clip: &ExtractedClipData,
    app_state: &Arc<AppState>,
) {
    let qdrant_client = match &app_state.qdrant_client {
        Some(c) => c,
        None => return,
    };

    let text_to_embed = format!("{} {}", clip.ai_title, clip.ai_description);

    let embedding = if let Some(ref voyage) = app_state.voyage_embeddings {
        voyage.generate_single_embedding(text_to_embed).await.ok()
    } else if let Some(ref gemini) = app_state.gemini_client {
        gemini
            .embed_content(&format!("{} {}", clip.ai_title, clip.ai_description))
            .await
            .ok()
    } else {
        None
    };

    let embedding = match embedding {
        Some(e) => e,
        None => return,
    };

    let _ = qdrant_client.ensure_extracted_clips_collection().await;

    let payload = serde_json::json!({
        "clip_id": clip_id,
        "clipping_job_id": job_id,
        "source_video_id": source_video_id,
        "title": clip.ai_title,
        "hook": clip.ai_description,
        "start_sec": clip.start_time_seconds,
        "end_sec": clip.end_time_seconds,
        "duration_sec": clip.duration_seconds,
        "quality_score": clip.ai_confidence_score,
        "viral_factors": clip.viral_factors,
        "upload_status": "uploaded",
        "uploaded_at": chrono::Utc::now().to_rfc3339(),
    });

    if let Err(e) = qdrant_client
        .store_extracted_clip(clip_id, payload, embedding)
        .await
    {
        tracing::debug!(
            "Failed to store clip {} in Qdrant (non-fatal): {}",
            clip_id,
            e
        );
    }
}

/// Infer the safest clipping resume point from durable workflow node state.
pub async fn infer_clipping_resume_from_nodes(
    pool: &PgPool,
    workflow_id: Option<uuid::Uuid>,
) -> Result<Option<String>, String> {
    let Some(workflow_id) = workflow_id else {
        return Ok(None);
    };

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT node_key, status
         FROM app_workflow_nodes
         WHERE workflow_id = $1
           AND node_key = ANY($2)",
    )
    .bind(workflow_id)
    .bind(vec![
        "analysis",
        "download",
        "extract_clips",
        "vectorize",
        "post_to_youtube",
    ])
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to infer clipping resume state from workflow nodes: {e}"))?;

    let has = |node: &str, statuses: &[&str]| -> bool {
        rows.iter()
            .any(|(node_key, status)| node_key == node && statuses.iter().any(|s| status == s))
    };

    if has("post_to_youtube", &["running", "failed", "completed"])
        || has("vectorize", &["completed"])
    {
        return Ok(Some("clips_extracted".to_string()));
    }

    if has("download", &["completed"])
        || has("extract_clips", &["running", "failed", "completed"])
        || has("vectorize", &["running", "failed"])
    {
        return Ok(Some("downloaded".to_string()));
    }

    if has("analysis", &["completed"]) || has("download", &["running", "failed"]) {
        return Ok(Some("analyzed".to_string()));
    }

    Ok(None)
}

// Helper functions

async fn ensure_clipping_workflow_plan(
    app_state: &Arc<AppState>,
    workflow_id: Option<uuid::Uuid>,
    job_id: i32,
    source_video_id: &str,
    video_url: &str,
) {
    let Some(workflow_id) = workflow_id else {
        return;
    };

    let runtime = crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
    let node_specs = [
        (
            "analysis",
            "gemini_video_analysis",
            json!({
                "job_id": job_id,
                "source_video_id": source_video_id,
                "video_url": video_url,
            }),
        ),
        (
            "download",
            "source_download",
            json!({
                "job_id": job_id,
                "source_video_id": source_video_id,
                "video_url": video_url,
            }),
        ),
        (
            "extract_clips",
            "clip_extraction",
            json!({
                "job_id": job_id,
                "source_video_id": source_video_id,
            }),
        ),
        (
            "vectorize",
            "source_vectorization",
            json!({
                "job_id": job_id,
                "source_video_id": source_video_id,
                "model_lane": "gemini_multimodal_or_configured_embedding",
            }),
        ),
        (
            "post_to_youtube",
            "youtube_upload",
            json!({
                "job_id": job_id,
                "source_video_id": source_video_id,
            }),
        ),
        (
            "fallback_summary",
            "generated_summary_delivery",
            json!({
                "job_id": job_id,
                "source_video_id": source_video_id,
                "video_url": video_url,
                "activation": "only_after_source_download_failure",
            }),
        ),
    ];

    for (node_key, node_type, input) in node_specs {
        let _ = runtime
            .ensure_node(workflow_id, node_key, node_type, input, 3)
            .await;
    }
}

async fn start_clipping_node(
    app_state: &Arc<AppState>,
    workflow_id: Option<uuid::Uuid>,
    node_key: &str,
    node_type: &str,
    input: serde_json::Value,
    message: &str,
) {
    let Some(workflow_id) = workflow_id else {
        return;
    };
    let runtime = crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
    let _ = runtime
        .ensure_node(workflow_id, node_key, node_type, input.clone(), 3)
        .await;
    let _ = runtime
        .start_node(workflow_id, node_key, message, input)
        .await;
}

async fn complete_clipping_node(
    app_state: &Arc<AppState>,
    workflow_id: Option<uuid::Uuid>,
    node_key: &str,
    output: serde_json::Value,
    message: &str,
) {
    let Some(workflow_id) = workflow_id else {
        return;
    };
    let runtime = crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
    let _ = runtime
        .complete_node(workflow_id, node_key, output, message)
        .await;
}

async fn fail_clipping_node(
    app_state: &Arc<AppState>,
    workflow_id: Option<uuid::Uuid>,
    node_key: &str,
    error_message: &str,
    details: serde_json::Value,
) {
    let Some(workflow_id) = workflow_id else {
        return;
    };
    let runtime = crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
    let _ = runtime
        .fail_node(workflow_id, node_key, error_message, details)
        .await;
}

async fn skip_clipping_node(
    app_state: &Arc<AppState>,
    workflow_id: Option<uuid::Uuid>,
    node_key: &str,
    reason: &str,
) {
    let Some(workflow_id) = workflow_id else {
        return;
    };
    let runtime = crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
    let _ = runtime
        .skip_node(workflow_id, node_key, reason, json!({ "reason": reason }))
        .await;
}

fn should_pause_after_clipping_node() -> bool {
    std::env::var("CLIPPING_NODE_STEP_MODE")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off" | "disabled"
            )
        })
        .unwrap_or(true)
}

async fn pause_clipping_after_node(
    app_state: &Arc<AppState>,
    job_id: i32,
    workflow_id: Option<uuid::Uuid>,
    completed_node: &str,
    resume_from: &str,
    progress: i32,
    message: &str,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE clipping_jobs
         SET status = 'pending',
             progress_percent = $2,
             current_step = $3,
             resume_from = $4,
             claimed_by = NULL,
             claimed_at = NULL,
             worker_heartbeat_at = NULL,
             updated_at = NOW()
         WHERE id = $1",
    )
    .bind(job_id)
    .bind(progress)
    .bind(completed_node)
    .bind(resume_from)
    .execute(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to pause clipping job after {completed_node}: {e}"))?;

    if let Some(workflow_id) = workflow_id {
        let runtime = crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
        let _ = runtime
            .mark_retrying(workflow_id, Some(completed_node), 0, message)
            .await;
        let _ = runtime
            .append_event(
                workflow_id,
                "node_checkpoint",
                Some(completed_node),
                message,
                json!({
                    "job_id": job_id,
                    "resume_from": resume_from,
                    "progress_percent": progress,
                    "mode": "clipping_node_step_mode",
                }),
            )
            .await;
    }

    tracing::info!(
        "Clipping job {} paused after node '{}' and requeued with resume_from='{}'",
        job_id,
        completed_node,
        resume_from
    );

    Ok(())
}

pub async fn fetch_job_details(job_id: i32, pool: &PgPool) -> Result<ClippingJob, String> {
    sqlx::query_as::<_, ClippingJob>("SELECT * FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to fetch job: {}", e))
}

pub async fn fetch_linkage(linkage_id: i32, pool: &PgPool) -> Result<ChannelLinkage, String> {
    sqlx::query_as::<_, ChannelLinkage>("SELECT * FROM youtube_channel_linkages WHERE id = $1")
        .bind(linkage_id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to fetch linkage: {}", e))
}

pub async fn fetch_destination_channel(
    channel_id: i32,
    pool: &PgPool,
) -> Result<ConnectedYouTubeChannel, String> {
    sqlx::query_as::<_, ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1",
    )
    .bind(channel_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to fetch destination channel: {}", e))
}

pub async fn load_reusable_source_analysis(
    current_job_id: i32,
    source_video_id: &str,
    pool: &PgPool,
) -> Result<Option<VideoAnalysis>, String> {
    let row = sqlx::query(
        "SELECT viral_moments_json
         FROM clipping_jobs
         WHERE source_video_id = $1
           AND id <> $2
           AND viral_moments_json IS NOT NULL
           AND COALESCE(analysis_quality, 0) >= 0.6
         ORDER BY
           CASE WHEN status = 'completed' THEN 0 ELSE 1 END,
           analysis_quality DESC NULLS LAST,
           updated_at DESC
         LIMIT 1",
    )
    .bind(source_video_id)
    .bind(current_job_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to load reusable source analysis: {}", e))?;

    let Some(row) = row else {
        return Ok(None);
    };

    let value: serde_json::Value = row
        .try_get("viral_moments_json")
        .map_err(|e| format!("Reusable source analysis missing payload: {}", e))?;
    let analysis = serde_json::from_value::<VideoAnalysis>(value)
        .map_err(|e| format!("Failed to deserialize reusable source analysis: {}", e))?;

    Ok(Some(analysis))
}

pub async fn persist_job_analysis(
    job_id: i32,
    analysis: &VideoAnalysis,
    pool: &PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE clipping_jobs
         SET viral_moments_json = $1, analysis_quality = $2, updated_at = NOW()
         WHERE id = $3",
    )
    .bind(serde_json::to_value(analysis).unwrap_or(serde_json::Value::Null))
    .bind(analysis.overall_quality)
    .bind(job_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to persist video analysis: {}", e))?;

    Ok(())
}

pub async fn store_source_analysis_vector(
    source_video_id: &str,
    video_url: &str,
    user_id: Option<i32>,
    channel_id: Option<&str>,
    analysis: &VideoAnalysis,
    app_state: &Arc<AppState>,
    phase: &str,
) {
    match VideoVectorizationService::store_video_analysis_from_gemini(
        source_video_id,
        video_url,
        user_id,
        channel_id,
        analysis,
        app_state,
    )
    .await
    {
        Ok(()) => tracing::info!(
            "✅ {}: source video_content stored/reused for {}",
            phase,
            source_video_id
        ),
        Err(e) => tracing::warn!(
            "{} source vectorization failed (non-fatal) for {}: {}",
            phase,
            source_video_id,
            e
        ),
    }
}

pub async fn update_job_status(
    job_id: i32,
    status: &str,
    progress: i32,
    error: Option<&str>,
    pool: &PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE clipping_jobs
         SET status = $1, progress_percent = $2, current_step = $1, error_message = $3, updated_at = NOW()
         WHERE id = $4",
    )
    .bind(status)
    .bind(progress)
    .bind(error)
    .bind(job_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update job status: {}", e))?;

    Ok(())
}

async fn update_job_video_path(job_id: i32, video_path: &str, pool: &PgPool) -> Result<(), String> {
    sqlx::query("UPDATE clipping_jobs SET local_video_path = $1, started_at = $2 WHERE id = $3")
        .bind(video_path)
        .bind(Utc::now())
        .bind(job_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to update video path: {}", e))?;

    Ok(())
}

pub async fn mark_job_completed(job_id: i32, pool: &PgPool) -> Result<(), String> {
    sqlx::query("UPDATE clipping_jobs SET completed_at = $1 WHERE id = $2")
        .bind(Utc::now())
        .bind(job_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to mark job completed: {}", e))?;

    Ok(())
}

pub async fn update_linkage_session_timestamp(
    linkage_id: i32,
    pool: &PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE youtube_channel_linkages SET last_clipping_session_at = NOW() WHERE id = $1",
    )
    .bind(linkage_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update session timestamp: {}", e))?;

    Ok(())
}

pub async fn handle_download_failure_fallback(
    job: &ClippingJob,
    linkage: &ChannelLinkage,
    original_video_url: &str,
    analysis: &VideoAnalysis,
    app_state: &Arc<AppState>,
    failure_reason: &str,
) -> Result<String, String> {
    let top_moments = analysis
        .top_moments(3)
        .into_iter()
        .enumerate()
        .map(|(index, moment)| {
            format!(
                "{}. {} — hook: {} — why it works: {}",
                index + 1,
                moment.title,
                moment.hook,
                moment.reason
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let fallback_prompt = format!(
        "Create a polished narrated fallback summary video based on this source content.\n\nVideo summary:\n{}\n\nHighest-value moments to emphasize:\n{}\n\nThe original source video could not be downloaded for clipping, so turn the topic into a standalone animated short that preserves the main ideas and hooks without requiring the raw footage. Use segmented motion graphics, concise narration, and visual variety from the available creative tools.",
        analysis.video_summary,
        top_moments
    );

    let delivery_title = format!(
        "Fallback summary delivery for {}",
        job.source_video_title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&job.source_video_id)
    );

    let extra_args = json!({
        "service_offer": "clipper_enhancement_pack",
        "source_url": original_video_url,
        "sample_owner_user_id": linkage.user_id,
        "include_narration": true,
        "narration_text": analysis.video_summary,
        "visual_direction": format!(
            "Fallback delivery generated after clipping download failure. Emphasize the strongest moments: {}",
            analysis
                .top_moments(3)
                .into_iter()
                .map(|moment| moment.title.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        "fallback_reason": failure_reason,
        "source_video_id": job.source_video_id,
        "source_video_title": job.source_video_title,
        "analysis_quality": analysis.overall_quality,
        "workflow_engine": "long_form_video_assembly",
        "target_duration_seconds": 60.0,
        "segment_duration_seconds": 15.0,
    });

    let delivery_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO deliveries
         (client_ref, title, gig_type, prompt, style, duration, extra_args, status, source_url)
         VALUES ($1, $2, 'long_form_video', $3, 'cinematic animated summary', 60.0, $4, 'pending', $5)
         RETURNING id",
    )
    .bind(format!("clipping-fallback:{}", job.id))
    .bind(&delivery_title)
    .bind(&fallback_prompt)
    .bind(&extra_args)
    .bind(original_video_url)
    .fetch_one(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to create fallback delivery: {}", e))?;

    let workflow_id = crate::services::LongFormVideoWorkflow::start(
        app_state.clone(),
        crate::services::LongFormVideoRequest {
            title: delivery_title.clone(),
            brief: fallback_prompt.clone(),
            target_duration_seconds: 60.0,
            segment_duration_seconds: 15.0,
            style: "cinematic animated summary, high-retention YouTube short, bold motion graphics"
                .to_string(),
            offer_type: "fallback_summary".to_string(),
            narration_speaker: "Emma".to_string(),
            include_narration: true,
            reference_url: Some(original_video_url.to_string()),
            session_uuid: None,
            user_id: Some(linkage.user_id),
            source_table: Some("deliveries".to_string()),
            source_record_id: Some(delivery_id),
            idempotency_key: Some(format!("clipping-fallback-long-form:{}", job.id)),
        },
    )
    .await?;

    sqlx::query("UPDATE deliveries SET workflow_id = $1 WHERE id = $2")
        .bind(workflow_id)
        .bind(delivery_id)
        .execute(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to attach fallback delivery workflow: {}", e))?;

    sqlx::query(
        "UPDATE clipping_jobs
         SET status = 'fallback_rendering',
             progress_percent = 85,
             current_step = 'fallback_delivery_queued',
             error_message = $1,
             fallback_delivery_id = $2,
             fallback_strategy = 'generated_summary_delivery',
             fallback_activated_at = NOW(),
             completed_at = NULL,
             updated_at = NOW()
         WHERE id = $3",
    )
    .bind(format!(
        "Source download failed; created fallback delivery workflow instead: {}",
        failure_reason
    ))
    .bind(delivery_id)
    .bind(job.id)
    .execute(&app_state.db_pool)
    .await
    .map_err(|e| {
        format!(
            "Failed to update clipping job with fallback delivery: {}",
            e
        )
    })?;

    Ok(format!(
        "Clipping download failed, so a segmented fallback summary delivery was created and queued. Delivery ID: {}. Workflow ID: {}.",
        delivery_id,
        workflow_id
    ))
}

fn clipping_download_timeout_secs() -> u64 {
    std::env::var("CLIPPING_DOWNLOAD_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(900)
}

pub(crate) async fn enhance_clips_with_full_agent(
    job_id: i32,
    user_id: i32,
    source_video_url: &str,
    content_type: &str,
    clips: &mut Vec<ExtractedClipData>,
    app_state: &Arc<AppState>,
    source_type: &str,
) {
    let enhancer = crate::clipping::clip_enhancer::ClipEnhancer::new(app_state.clone());

    for clip in clips.iter_mut() {
        enhancer.enhance_clip(clip, content_type).await;

        let mut review =
            review_clip_with_qa(clip, app_state, source_type, source_video_url, content_type).await;
        let mut reviewed_after_retry = false;

        if !review.pass {
            if let Some(retry_hint) = review.retry_hint.as_deref() {
                tracing::warn!(
                    job_id = job_id,
                    clip_number = clip.clip_number,
                    score = review.score,
                    retry_hint = retry_hint,
                    "Clip QA failed; retrying enhancement once with reviewer hint recorded"
                );
                append_clip_retry_hint(clip, retry_hint);
                enhancer.enhance_clip(clip, content_type).await;
                review = review_clip_with_qa(
                    clip,
                    app_state,
                    source_type,
                    source_video_url,
                    content_type,
                )
                .await;
                reviewed_after_retry = true;
            }
        }

        apply_review_to_clip(clip, &review, reviewed_after_retry);
        persist_clip_review_artifact(
            job_id,
            user_id,
            source_video_url,
            clip,
            app_state,
            source_type,
        )
        .await;
    }
}

fn append_clip_retry_hint(clip: &mut ExtractedClipData, retry_hint: &str) {
    let merged = match clip.enhancement_reasoning.take() {
        Some(existing) if !existing.trim().is_empty() => {
            format!("{existing}\nQA retry hint: {retry_hint}")
        }
        _ => format!("QA retry hint: {retry_hint}"),
    };
    clip.enhancement_reasoning = Some(merged);
}

async fn review_clip_with_qa(
    clip: &ExtractedClipData,
    app_state: &Arc<AppState>,
    source_type: &str,
    source_video_url: &str,
    content_type: &str,
) -> crate::render_review::ReviewResult {
    let prompt = format!(
        "Review this {source_type} short clip.\nTitle: {title}\nDescription: {description}\nSource URL: {source_video_url}\nContent type: {content_type}\nEnhancement notes: {enhancement}",
        source_type = source_type,
        title = clip.ai_title,
        description = clip.ai_description,
        source_video_url = source_video_url,
        content_type = content_type,
        enhancement = clip.enhancement_reasoning.as_deref().unwrap_or("none"),
    );

    crate::render_review::review_render(
        app_state,
        &clip.local_clip_path,
        &prompt,
        "clip_enhancement",
        None,
    )
    .await
}

fn apply_review_to_clip(
    clip: &mut ExtractedClipData,
    review: &crate::render_review::ReviewResult,
    reviewed_after_retry: bool,
) {
    let status = if review.score <= 0 {
        "review_skipped"
    } else if review.pass && reviewed_after_retry {
        "passed_after_retry"
    } else if review.pass {
        "passed"
    } else if reviewed_after_retry {
        "warning_after_retry"
    } else {
        "warning"
    };

    clip.qa_status = Some(status.to_string());
    clip.qa_score = Some(review.score);
    clip.qa_feedback = Some(review.feedback.clone());
    clip.qa_retry_hint = review.retry_hint.clone();
}

async fn persist_clip_review_artifact(
    job_id: i32,
    user_id: i32,
    source_video_url: &str,
    clip: &ExtractedClipData,
    app_state: &Arc<AppState>,
    source_type: &str,
) {
    let artifact = crate::services::media_review::MediaReviewArtifact {
        review_id: format!("{source_type}-{job_id}-{}", clip.clip_number),
        asset_kind: "clip".to_string(),
        source_type: source_type.to_string(),
        service_slug: Some("clipper-enhancement-pack".to_string()),
        owner_user_id: Some(user_id),
        output_url: Some(clip.local_clip_path.clone()),
        source_url: Some(source_video_url.to_string()),
        prompt: Some(format!("{} {}", clip.ai_title, clip.ai_description)),
        title: Some(clip.ai_title.clone()),
        company: None,
        review_status: clip
            .qa_status
            .clone()
            .unwrap_or_else(|| "not_reviewed".to_string()),
        qa_score: clip.qa_score,
        qa_feedback: clip.qa_feedback.clone(),
        narration_text: None,
        visual_direction: clip.enhancement_reasoning.clone(),
        transcript_excerpt: Some(clip.ai_description.clone()),
        tags: vec![
            "clip".to_string(),
            source_type.to_string(),
            format!("clip_number_{}", clip.clip_number),
        ],
    };

    if let Err(error) =
        crate::services::media_review::MediaReviewService::store_artifact(app_state, artifact).await
    {
        tracing::warn!(
            job_id = job_id,
            clip_number = clip.clip_number,
            error = %error,
            "Failed to store clip media review artifact"
        );
    }
}

pub async fn save_clips_to_database(
    job_id: i32,
    clips: &[ExtractedClipData],
    linkage: &ChannelLinkage,
    pool: &PgPool,
) -> Result<Vec<i32>, String> {
    // Delete any stale clips from a previous failed attempt for this job.
    // Without this, retried jobs accumulate duplicate extracted_clips rows.
    sqlx::query("DELETE FROM extracted_clips WHERE clipping_job_id = $1")
        .bind(job_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to clean stale clips before save: {}", e))?;

    let mut clip_ids = Vec::new();

    for clip in clips {
        let clip_id: i32 = sqlx::query_scalar(
            "INSERT INTO extracted_clips
             (clipping_job_id, clip_number, local_clip_path,
             start_time_seconds, end_time_seconds, duration_seconds,
              ai_title, ai_description, ai_tags, ai_confidence_score, viral_factors,
              destination_channel_id, custom_thumbnail_path, thumbnail_generation_method,
              enhancement_applied, enhancement_tools, enhancement_reasoning,
              r2_clip_key, r2_thumb_key, r2_clip_url,
              qa_status, qa_score, qa_feedback, qa_retry_hint)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24)
             RETURNING id",
        )
        .bind(job_id)
        .bind(clip.clip_number)
        .bind(&clip.local_clip_path)
        .bind(clip.start_time_seconds)
        .bind(clip.end_time_seconds)
        .bind(clip.duration_seconds)
        .bind(&clip.ai_title)
        .bind(&clip.ai_description)
        .bind(&clip.ai_tags)
        .bind(clip.ai_confidence_score)
        .bind(&clip.viral_factors)
        .bind(linkage.destination_channel_id)
        .bind(&clip.custom_thumbnail_path)
        .bind(&clip.thumbnail_generation_method)
        .bind(clip.enhancement_applied)
        .bind(&clip.enhancement_tools)
        .bind(&clip.enhancement_reasoning)
        .bind(&clip.r2_clip_key)
        .bind(&clip.r2_thumb_key)
        .bind(&clip.r2_clip_url)
        .bind(clip.qa_status.as_deref().unwrap_or("not_reviewed"))
        .bind(clip.qa_score)
        .bind(&clip.qa_feedback)
        .bind(&clip.qa_retry_hint)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to save clip: {}", e))?;

        clip_ids.push(clip_id);
    }

    Ok(clip_ids)
}

/// Count clips successfully published for a destination channel in the last 24 hours.
pub async fn count_clips_posted_today(
    destination_channel_id: i32,
    pool: &PgPool,
) -> Result<i64, String> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM extracted_clips
         WHERE destination_channel_id = $1
           AND upload_status = 'published'
           AND published_at >= NOW() - INTERVAL '24 hours'",
    )
    .bind(destination_channel_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to count daily clips: {}", e))
}

pub async fn update_linkage_stats(
    linkage_id: i32,
    clips_generated: i32,
    clips_posted: i32,
    pool: &PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE youtube_channel_linkages
         SET total_clips_generated = total_clips_generated + $1,
             total_clips_posted = total_clips_posted + $2,
             last_clip_generated_at = NOW()
         WHERE id = $3",
    )
    .bind(clips_generated)
    .bind(clips_posted)
    .bind(linkage_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update linkage stats: {}", e))?;

    Ok(())
}
