//! Manual Clipping Integration Test Runner
//!
//! Triggered from admin dashboard via POST /api/admin/manual-clipping-tests/trigger
//!
//! Phase 1 — Prospect Discovery: uses YouTube Data API to find real, public video URLs
//!           from content creators in gaming, tech, and fitness niches. This validates
//!           that the prospect finder can locate actual target clients.
//!
//! Phase 2 — Manual Clipping: runs the full pipeline on each discovered video URL
//!           (Gemini analysis → download via Apify → FFmpeg extraction → R2 upload).
//!
//! Phase 3 — AI Review: Gemini evaluates clip metadata and R2 storage success.
//!
//! Results are recorded in the `test_runs` / `test_results` tables and visible
//! at /admin/test-runs in the admin dashboard.

use crate::AppState;
use serde_json::Value;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Niche search queries for Phase 1
// ─────────────────────────────────────────────────────────────────────────────

struct SearchQuery {
    keyword: &'static str,
    niche: &'static str,
    description: &'static str,
}

fn search_queries() -> Vec<SearchQuery> {
    vec![
        SearchQuery {
            keyword: "gaming highlights best moments compilation",
            niche: "gaming",
            description:
                "Gaming channel — best-moment clip extraction service for content creators",
        },
        SearchQuery {
            keyword: "tech tutorial explainer short",
            niche: "tech",
            description:
                "Tech tutorial channel — educational clip extraction for YouTube Shorts pipeline",
        },
        SearchQuery {
            keyword: "fitness workout highlights motivation",
            niche: "fitness",
            description: "Fitness channel — workout highlight reel automatic clipping demo",
        },
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// Discovered scenario — produced by Phase 1, consumed by Phase 2
// ─────────────────────────────────────────────────────────────────────────────

struct ClipScenario {
    name: String,
    niche: String,
    description: String,
    video_url: String,
    clips_requested: i32,
    min_dur: i32,
    max_dur: i32,
}

// ─────────────────────────────────────────────────────────────────────────────
// LLM review helpers (identical pattern to portfolio_tests.rs)
// ─────────────────────────────────────────────────────────────────────────────

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

// ─────────────────────────────────────────────────────────────────────────────
// Runner
// ─────────────────────────────────────────────────────────────────────────────

pub struct ManualClippingTestRunner {
    app_state: Arc<AppState>,
}

impl ManualClippingTestRunner {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Insert a test_run row and spawn the full test suite in the background.
    /// Returns the new run's UUID immediately so the admin can track progress.
    pub async fn create_and_spawn(app_state: Arc<AppState>, name: String) -> Result<Uuid, String> {
        let row = sqlx::query("INSERT INTO test_runs (name) VALUES ($1) RETURNING id")
            .bind(&name)
            .fetch_one(&app_state.db_pool)
            .await
            .map_err(|e| format!("Failed to create test run: {e}"))?;

        let run_id: Uuid = row.get("id");

        tokio::spawn(async move {
            let runner = ManualClippingTestRunner::new(app_state);
            runner.run_all(run_id).await;
        });

        Ok(run_id)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Orchestration
    // ─────────────────────────────────────────────────────────────────────────

    async fn run_all(&self, run_id: Uuid) {
        tracing::info!(
            "🎬 Manual clipping test run {} — Phase 1: video discovery",
            run_id
        );

        // Phase 1 — discover real video URLs via YouTube Data API
        let scenarios = match self.discover_videos().await {
            Ok(s) if !s.is_empty() => s,
            Ok(_) => {
                self.finish_run(run_id, "failed").await;
                tracing::error!("Phase 1 returned 0 videos — aborting run {}", run_id);
                return;
            }
            Err(e) => {
                self.finish_run(run_id, "failed").await;
                tracing::error!("Phase 1 failed for run {}: {e}", run_id);
                return;
            }
        };

        let total = scenarios.len() as i32;
        let _ = sqlx::query("UPDATE test_runs SET total_tests = $1 WHERE id = $2")
            .bind(total)
            .bind(run_id)
            .execute(&self.app_state.db_pool)
            .await;

        tracing::info!(
            "🎬 Manual clipping test run {} — Phase 2: clipping {} videos",
            run_id,
            total
        );

        // Phase 2+3 — clip each discovered video and record results
        // Space tests 3 minutes apart so Gemini per-minute quota can partially recover
        // between sequential video-analysis calls that each consume a Gemini permit.
        for (i, scenario) in scenarios.iter().enumerate() {
            if i > 0 {
                tracing::info!("⏳ Waiting 3 min before next test to relieve Gemini quota…");
                tokio::time::sleep(std::time::Duration::from_secs(180)).await;
            }
            self.run_one(run_id, scenario).await;
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

        tracing::info!("✅ Manual clipping test run {} complete", run_id);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 1 — YouTube Data API video discovery
    // ─────────────────────────────────────────────────────────────────────────

    async fn discover_videos(&self) -> Result<Vec<ClipScenario>, String> {
        let api_key =
            std::env::var("YOUTUBE_API_KEY").map_err(|_| "YOUTUBE_API_KEY not configured")?;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("HTTP client build failed: {e}"))?;

        let queries = search_queries();
        let mut scenarios = Vec::new();

        for q in &queries {
            // videoDuration=medium → 4–20 min clips, manageable for the test
            let search_url = format!(
                "https://www.googleapis.com/youtube/v3/search\
                 ?part=snippet&type=video&maxResults=5&order=viewCount\
                 &videoDuration=medium&safeSearch=moderate\
                 &q={}&key={}",
                urlencoding::encode(q.keyword),
                api_key
            );

            let resp = match client.get(&search_url).send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("YouTube search failed for '{}': {e}", q.keyword);
                    continue;
                }
            };

            if !resp.status().is_success() {
                tracing::warn!("YouTube API {} for query '{}'", resp.status(), q.keyword);
                continue;
            }

            let json: Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!("YouTube API JSON parse error: {e}");
                    continue;
                }
            };

            let items = json
                .get("items")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            // Pick the first public video
            let mut found = false;
            for item in &items {
                let video_id = item
                    .pointer("/id/videoId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                if video_id.is_empty() {
                    continue;
                }

                let title = item
                    .pointer("/snippet/title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string();

                let video_url = format!("https://www.youtube.com/watch?v={video_id}");
                let short_title: String = title.chars().take(60).collect();

                tracing::info!(
                    "Discovered [{niche}] video: {title} → {url}",
                    niche = q.niche,
                    title = short_title,
                    url = video_url
                );

                scenarios.push(ClipScenario {
                    name: format!("[{}] {}", q.niche.to_uppercase(), short_title),
                    niche: q.niche.to_string(),
                    description: format!(
                        "{} — video '{}' discovered via YouTube API prospect search",
                        q.description, short_title
                    ),
                    video_url,
                    clips_requested: 3,
                    min_dur: 30,
                    max_dur: 90,
                });

                found = true;
                break; // one video per niche query
            }

            if !found {
                tracing::warn!("No usable video found for niche '{}'", q.niche);
            }
        }

        tracing::info!("Phase 1 complete — discovered {} video(s)", scenarios.len());
        Ok(scenarios)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 2+3 — run pipeline on one video and record results
    // ─────────────────────────────────────────────────────────────────────────

    async fn run_one(&self, run_id: Uuid, scenario: &ClipScenario) {
        tracing::info!("▶ Manual clip test: {}", scenario.name);

        // Insert test_results row
        let row = match sqlx::query(
            "INSERT INTO test_results (run_id, test_name, gig_type, prompt, status) \
             VALUES ($1, $2, 'manual_clipping', $3, 'running') RETURNING id",
        )
        .bind(run_id)
        .bind(&scenario.name)
        .bind(&scenario.description)
        .fetch_one(&self.app_state.db_pool)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("DB insert for test_results failed: {e}");
                return;
            }
        };
        let result_id: Uuid = row.get("id");

        // Resolve a real user_id to satisfy the FK — use first superuser
        let user_id: i32 = match sqlx::query(
            "SELECT id FROM users WHERE is_superuser = true ORDER BY id ASC LIMIT 1",
        )
        .fetch_optional(&self.app_state.db_pool)
        .await
        {
            Ok(Some(r)) => r.get("id"),
            _ => {
                // Fall back to first active user
                match sqlx::query(
                    "SELECT id FROM users WHERE is_active = true ORDER BY id ASC LIMIT 1",
                )
                .fetch_optional(&self.app_state.db_pool)
                .await
                {
                    Ok(Some(r)) => r.get("id"),
                    _ => {
                        let msg = "No users in DB — cannot create test job";
                        tracing::error!("{msg}");
                        self.fail_result(result_id, run_id, msg).await;
                        return;
                    }
                }
            }
        };

        // Insert a manual_clipping_jobs row for this test
        let platform = if scenario.video_url.contains("twitch.tv") {
            "twitch"
        } else {
            "youtube"
        };

        let job_row = match sqlx::query(
            "INSERT INTO manual_clipping_jobs \
             (user_id, video_url, video_platform, clips_requested, \
              min_clip_duration_seconds, max_clip_duration_seconds) \
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        )
        .bind(user_id)
        .bind(&scenario.video_url)
        .bind(platform)
        .bind(scenario.clips_requested)
        .bind(scenario.min_dur)
        .bind(scenario.max_dur)
        .fetch_one(&self.app_state.db_pool)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("Failed to create job row: {e}");
                tracing::error!("{msg}");
                self.fail_result(result_id, run_id, &msg).await;
                return;
            }
        };
        let job_id: Uuid = job_row.get("id");

        // Phase 2: run the full manual clipping pipeline
        match crate::jobs::manual_clipping_job::execute_manual_clipping_job(
            job_id,
            self.app_state.clone(),
        )
        .await
        {
            Ok(summary) => {
                // Collect clip metadata from DB
                let clips = sqlx::query(
                    "SELECT r2_clip_url, r2_clip_key, title, duration_seconds, quality_score \
                     FROM manual_clipping_clips WHERE job_id = $1 ORDER BY clip_number ASC",
                )
                .bind(job_id)
                .fetch_all(&self.app_state.db_pool)
                .await
                .unwrap_or_default();

                let first_url = clips
                    .first()
                    .and_then(|r| r.get::<Option<String>, _>("r2_clip_url"))
                    .unwrap_or_default();

                let first_key = clips
                    .first()
                    .and_then(|r| r.get::<Option<String>, _>("r2_clip_key"))
                    .unwrap_or_default();

                // Phase 3: Gemini quality review
                let review = self.review_clips(scenario, &clips).await;

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
                .bind(&first_key)
                .bind(&first_url)
                .bind(format!("{}_clips.mp4", clips.len()))
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

                tracing::info!(
                    "✅ '{}' passed — {} — Gemini score {}/10",
                    scenario.name,
                    summary,
                    review.score
                );
            }
            Err(e) => {
                tracing::error!("❌ '{}' failed: {e}", scenario.name);
                self.fail_result(result_id, run_id, &e).await;
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 3 — Gemini clip quality review
    // ─────────────────────────────────────────────────────────────────────────

    async fn review_clips(
        &self,
        scenario: &ClipScenario,
        clips: &[sqlx::postgres::PgRow],
    ) -> LLMReview {
        let gemini = match &self.app_state.gemini_client {
            Some(g) => g,
            None => {
                return LLMReview {
                    score: 0,
                    feedback: "Gemini not configured — review skipped".to_string(),
                }
            }
        };

        let clips_summary: Vec<String> = clips
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let dur = r.get::<Option<f64>, _>("duration_seconds").unwrap_or(0.0);
                let score = r.get::<Option<f64>, _>("quality_score").unwrap_or(0.0);
                let title = r
                    .get::<Option<String>, _>("title")
                    .unwrap_or_else(|| format!("Clip {}", i + 1));
                let has_url = r.get::<Option<String>, _>("r2_clip_url").is_some();
                format!(
                    "  • Clip {}: '{}' — {:.0}s, quality {:.0}%, R2 url: {}",
                    i + 1,
                    title,
                    dur,
                    score * 100.0,
                    if has_url { "YES" } else { "MISSING" }
                )
            })
            .collect();

        let prompt = format!(
            "You are a video clipping service quality reviewer assessing our AI pipeline \
             for Fiverr/PPH clients.\n\
             Source video: {url}\n\
             Niche: {niche}\n\
             Service pitch: {desc}\n\
             Pipeline result — {count} clip(s) generated:\n{clips}\n\n\
             Rate this result 1-10 as evidence that our automated clipping service \
             works reliably for real content. Key criteria: Did clips generate? \
             Are they stored in R2 (download links present)? Are quality scores \
             reasonable? Does this demonstrate value to a content creator client?\n\
             Reply ONLY as JSON: {{\"score\": <int 1-10>, \"feedback\": \"<2 sentences max>\"}}",
            url = scenario.video_url,
            niche = scenario.niche,
            desc = scenario.description,
            count = clips.len(),
            clips = if clips.is_empty() {
                "  (no clips generated)".to_string()
            } else {
                clips_summary.join("\n")
            },
        );

        match gemini.generate_text(&prompt).await {
            Ok(resp) => parse_review(&resp),
            Err(e) => LLMReview {
                score: 0,
                feedback: format!("Gemini review call failed: {e}"),
            },
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Helpers
    // ─────────────────────────────────────────────────────────────────────────

    async fn fail_result(&self, result_id: Uuid, run_id: Uuid, error: &str) {
        let _ = sqlx::query(
            "UPDATE test_results \
             SET status = 'failed', error_message = $1, completed_at = NOW() \
             WHERE id = $2",
        )
        .bind(error)
        .bind(result_id)
        .execute(&self.app_state.db_pool)
        .await;

        let _ = sqlx::query("UPDATE test_runs SET failed_tests = failed_tests + 1 WHERE id = $1")
            .bind(run_id)
            .execute(&self.app_state.db_pool)
            .await;
    }

    async fn finish_run(&self, run_id: Uuid, status: &str) {
        let _ = sqlx::query("UPDATE test_runs SET status = $1, completed_at = NOW() WHERE id = $2")
            .bind(status)
            .bind(run_id)
            .execute(&self.app_state.db_pool)
            .await;
    }
}
