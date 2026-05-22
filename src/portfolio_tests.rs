//! Portfolio test runner — Fiverr gig scenario tests that run server-side.
//!
//! Triggered from /admin/test-runs. Each run exercises all 7 Blender tools
//! (12 scenarios total), uploads outputs to R2 with 7-day presigned URLs,
//! and asks Gemini to review each result.  Results are stored in the
//! `test_runs` / `test_results` tables and surfaced in the admin dashboard.

use crate::AppState;
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Scenario catalogue
// ─────────────────────────────────────────────────────────────────────────────

struct Scenario {
    name: &'static str,
    gig_type: &'static str,
    description: &'static str,
    tool: &'static str,
    args: fn() -> Value,
    url_key: &'static str,
    ext: &'static str,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "LaTeX Animation — E=mc²",
            gig_type: "latex_math_animation",
            description: "Einstein equation with dark background, appear animation — core math gig showcase ($300-$800).",
            tool: "blender_generate_latex",
            args: || json!({
                "latex_expression": "E = mc^2",
                "animation_type": "appear",
                "duration": 5.0,
                "background_style": "dark"
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "LaTeX Animation — Gaussian Integral",
            gig_type: "latex_math_animation",
            description: "Step-by-step Gaussian integral proof animation — advanced math portfolio piece.",
            tool: "blender_generate_latex",
            args: || json!({
                "latex_expression": r"\int_{-\infty}^{\infty} e^{-x^2} dx = \sqrt{\pi}",
                "animation_type": "step_by_step",
                "duration": 8.0,
                "background_style": "dark"
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "3D Scene — Cinematic Landscape",
            gig_type: "3d_scene",
            description: "Cinematic mountain landscape at golden hour — demonstrates 3D scene generation.",
            tool: "blender_generate_scene",
            args: || json!({
                "prompt": "cinematic mountain landscape at golden hour sunset with dramatic volumetric lighting",
                "duration": 5.0,
                "style": "cinematic"
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "3D Scene — Abstract Tech Visualization",
            gig_type: "3d_scene",
            description: "Abstract futuristic network visualization — suitable for corporate/tech intros.",
            tool: "blender_generate_scene",
            args: || json!({
                "prompt": "abstract futuristic network of glowing blue nodes and data streams on dark background",
                "duration": 5.0,
                "style": "energetic"
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "YouTube Thumbnail — AI Tech",
            gig_type: "youtube_thumbnail",
            description: "3D rendered YouTube thumbnail for AI/tech topic — high-value gig ($40-$100).",
            tool: "blender_generate_thumbnail",
            args: || json!({
                "prompt": "futuristic artificial intelligence brain neural network dark theme neon blue glow",
                "title_text": "The Future of AI",
                "style": "youtube"
            }),
            url_key: "image_url",
            ext: "png",
        },
        Scenario {
            name: "YouTube Thumbnail — Finance",
            gig_type: "youtube_thumbnail",
            description: "3D rendered thumbnail for finance/crypto content — high CPM niche.",
            tool: "blender_generate_thumbnail",
            args: || json!({
                "prompt": "cryptocurrency bitcoin gold coins rising chart dark background dramatic lighting",
                "title_text": "10x Your Money in 2026",
                "style": "youtube"
            }),
            url_key: "image_url",
            ext: "png",
        },
        Scenario {
            name: "Animated Title Card — Tech Channel",
            gig_type: "title_card",
            description: "Corporate animated intro title card for tech YouTube channel.",
            tool: "blender_generate_title_card",
            args: || json!({
                "title": "Tech Weekly",
                "subtitle": "Your Weekly Dose of Innovation",
                "duration": 5.0,
                "style": "corporate"
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "Animated Title Card — Educational",
            gig_type: "title_card",
            description: "Clean educational-style title card for tutorial/course content.",
            tool: "blender_generate_title_card",
            args: || json!({
                "title": "Learn Python in 30 Days",
                "subtitle": "Episode 1: Getting Started",
                "duration": 5.0,
                "style": "minimal"
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "Data Visualization — Revenue Chart",
            gig_type: "data_visualization",
            description: "Animated bar chart for quarterly revenue data — B2B/corporate gig.",
            tool: "blender_generate_data_viz",
            args: || json!({
                "data_json": r#"[{"label":"Q1","value":42},{"label":"Q2","value":78},{"label":"Q3","value":55},{"label":"Q4","value":91}]"#,
                "chart_type": "bar",
                "title": "Quarterly Revenue Growth",
                "duration": 8.0
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "Lower Third — Broadcast Style",
            gig_type: "lower_third",
            description: "Professional broadcast-style animated lower third overlay.",
            tool: "blender_generate_lower_third",
            args: || json!({
                "name_text": "Dr. Sarah Chen",
                "subtitle_text": "AI Research Lead, Stanford University",
                "style": "corporate",
                "duration": 5.0
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "UI Mockup — iPhone App Demo",
            gig_type: "ui_mockup",
            description: "Animated iPhone mockup with reveal animation — for app promo videos.",
            tool: "blender_generate_ui_mockup",
            args: || json!({
                "device": "iPhone",
                "animation": "reveal",
                "duration": 5.0,
                "screenshot_url": ""
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "UI Mockup — MacBook Product Shot",
            gig_type: "ui_mockup",
            description: "MacBook mockup with tilt animation — used in SaaS product demos.",
            tool: "blender_generate_ui_mockup",
            args: || json!({
                "device": "MacBook",
                "animation": "tilt",
                "duration": 5.0,
                "screenshot_url": ""
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "Particle Confetti — Celebration Intro",
            gig_type: "celebration_animation",
            description: "Colourful confetti burst animation for YouTube celebrations, event promos, and milestones.",
            tool: "blender_generate_particle_confetti",
            args: || json!({
                "style": "confetti",
                "count": 400,
                "duration": 5.0,
                "primary_color": [1.0, 0.25, 0.1, 1.0],
                "secondary_color": [0.1, 0.4, 1.0, 1.0]
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "Rigid Body Drop — Logo Reveal",
            gig_type: "logo_animation",
            description: "3D physics-based logo letter drop — premium Fiverr motion graphics gig.",
            tool: "blender_generate_rigid_body_drop",
            args: || json!({
                "text": "BRAND",
                "object_type": "text",
                "style": "dark",
                "duration": 4.0
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "Camera Orbit — Abstract Product",
            gig_type: "cinematic_animation",
            description: "Smooth 360° camera orbit around abstract 3D scene — cinematic B-roll or product showcase.",
            tool: "blender_generate_camera_path",
            args: || json!({
                "path_type": "orbit",
                "subject": "abstract",
                "duration": 8.0,
                "style": "cinematic"
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "Toon Scene — Cartoon Robots",
            gig_type: "cartoon_animation",
            description: "NPR toon-shaded cartoon scene with robots — for kids content, explainers, and stylised brand videos.",
            tool: "blender_generate_toon_scene",
            args: || json!({
                "subject": "robots",
                "title": "Hello World",
                "flat_shading": true,
                "duration": 5.0
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "Whiteboard Reveal — Brand Name",
            gig_type: "whiteboard_animation",
            description: "Whiteboard draw-on animation of brand text — hugely popular on Fiverr for explainer videos.",
            tool: "blender_generate_grease_pencil_reveal",
            args: || json!({
                "text": "DEVTHUKU",
                "style": "whiteboard",
                "duration": 6.0,
                "stroke_width": 50
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "Geometry Scatter — Sphere Field",
            gig_type: "abstract_background",
            description: "Procedural animated sphere scatter across torus surface with wave displacement — abstract motion background.",
            tool: "blender_generate_geometry_scatter",
            args: || json!({
                "instance_type": "spheres",
                "surface": "torus",
                "count": 150,
                "animated": true,
                "duration": 7.0
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "Geometry Proof — Pythagorean",
            gig_type: "math_animation",
            description: "Animated Pythagorean theorem proof using Manim — popular for educational YouTube channels.",
            tool: "blender_generate_geometry_proof",
            args: || json!({
                "proof_type": "pythagorean",
                "title": "Pythagorean Theorem",
                "color_a": "BLUE",
                "color_b": "RED",
                "show_labels": true,
                "duration": 12.0
            }),
            url_key: "video_url",
            ext: "mp4",
        },
        Scenario {
            name: "Text Animation — Kinetic Title",
            gig_type: "kinetic_typography",
            description: "Manim wave kinetic typography animation — high-demand for social media reels and intros.",
            tool: "blender_generate_text_animation",
            args: || json!({
                "text": "YOUR BRAND",
                "mode": "wave",
                "color": "GOLD",
                "duration": 6.0,
                "font_size": 72
            }),
            url_key: "video_url",
            ext: "mp4",
        },
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// LLM review
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

pub struct PortfolioTestRunner {
    app_state: Arc<AppState>,
}

impl PortfolioTestRunner {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Create a new test_run row and spawn a background task that runs all scenarios.
    /// Returns the new run's UUID.
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
            // Brief pause between scenarios to avoid Gemini reviewer 429 rate limiting
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

        tracing::info!("Portfolio test run {run_id} finished");
    }

    async fn run_one(&self, run_id: Uuid, scenario: &Scenario) {
        tracing::info!("▶ Portfolio test: {}", scenario.name);

        let row = match sqlx::query(
            "INSERT INTO test_results (run_id, test_name, gig_type, prompt, status) \
             VALUES ($1, $2, $3, $4, 'running') RETURNING id",
        )
        .bind(run_id)
        .bind(scenario.name)
        .bind(scenario.gig_type)
        .bind(scenario.description)
        .fetch_one(&self.app_state.db_pool)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("DB insert failed for '{}': {e}", scenario.name);
                return;
            }
        };
        let result_id: Uuid = row.get("id");

        match self.execute(scenario).await {
            Ok((local_path, r2_url, r2_key, filename)) => {
                let review = self.review(&r2_url, &local_path, scenario).await;

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
                .bind(&r2_key)
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

    async fn execute(
        &self,
        scenario: &Scenario,
    ) -> Result<(String, String, String, String), String> {
        let client =
            self.app_state.blender_mcp_client.as_ref().ok_or_else(|| {
                "BlenderMCPClient not configured — set BLENDER_MCP_URL".to_string()
            })?;

        let local_path = client
            .render_async(
                scenario.tool,
                (scenario.args)(),
                scenario.url_key,
                scenario.ext,
            )
            .await?;

        let filename = local_path.split('/').last().unwrap_or("output").to_string();

        let r2_key = format!("portfolio-tests/{filename}");

        let (r2_url, final_key) = if let Some(r2) = &self.app_state.r2_client {
            r2.upload(&local_path, &r2_key)
                .await
                .map_err(|e| format!("R2 upload failed: {e}"))?;
            let url = r2
                .presign_get(&r2_key, 7 * 24 * 3600)
                .await
                .map_err(|e| format!("R2 presign failed: {e}"))?;
            (url, r2_key)
        } else {
            (format!("/outputs/{filename}"), "local".to_string())
        };

        Ok((local_path, r2_url, final_key, filename))
    }

    async fn review(&self, r2_url: &str, local_path: &str, scenario: &Scenario) -> LLMReview {
        let gemini = match self
            .app_state
            .video_gemini_client
            .as_ref()
            .or(self.app_state.gemini_client.as_ref())
        {
            Some(g) => g,
            None => {
                return LLMReview {
                    score: 0,
                    feedback: "Gemini not configured — review skipped".to_string(),
                }
            }
        };

        // For PNG thumbnails: pass actual image bytes for visual review
        if scenario.ext == "png" {
            if let Ok(bytes) = tokio::fs::read(local_path).await {
                let prompt = format!(
                    "You are a professional YouTube thumbnail reviewer. \
                     This 3D-rendered thumbnail was generated for: \"{}\"\n\n\
                     Rate it 1-10 (10=perfect) on: visual quality, title readability, \
                     click-worthiness, and portfolio suitability for freelance clients.\n\
                     Reply ONLY as JSON: {{\"score\": <int 1-10>, \"feedback\": \"<2 sentences max>\"}}",
                    scenario.description
                );
                if let Ok(response) = gemini.analyze_image_bytes(&bytes, &prompt).await {
                    return parse_review(&response);
                }
            }
        }

        // For video outputs: text-based quality assessment
        let uploaded = !r2_url.starts_with("/outputs/");
        let prompt = format!(
            "You are a professional video production reviewer assessing a Fiverr portfolio piece.\n\
             Gig type: {gig}\n\
             Brief: \"{desc}\"\n\
             Tool: {tool}\n\
             Render result: {status}\n\n\
             Based on this gig type and what automated 3D rendering can achieve, \
             rate the likely portfolio suitability 1-10 and give specific advice \
             on what this video demonstrates to potential buyers.\n\
             Reply ONLY as JSON: {{\"score\": <int 1-10>, \"feedback\": \"<2 sentences max>\"}}",
            gig = scenario.gig_type,
            desc = scenario.description,
            tool = scenario.tool,
            status = if uploaded {
                "rendered and uploaded to R2 successfully"
            } else {
                "rendered locally (no R2 upload)"
            },
        );

        match gemini.generate_text(&prompt).await {
            Ok(resp) => parse_review(&resp),
            Err(e) => LLMReview {
                score: 0,
                feedback: format!("Review call failed: {e}"),
            },
        }
    }
}
