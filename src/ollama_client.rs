/// Ollama client — self-hosted LLM via Ollama OpenAI-compatible API.
/// Default model: gemma4:12b (encoder-free multimodal, vision+audio, ~7.6GB).
/// Configurable via OLLAMA_BASE_URL and OLLAMA_MODEL env vars.
///
/// Vision support: uses OpenAI-compatible content parts array with
/// `image_url` parts (base64-encoded JPEG) for multimodal analysis.
use base64::prelude::*;
use reqwest::Client;

const OLLAMA_DEFAULT_URL: &str = "http://172.31.42.118:11434";
const OLLAMA_DEFAULT_MODEL: &str = "gemma4:12b";

#[derive(Debug, Clone)]
pub struct OllamaToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug)]
pub enum OllamaResponse {
    Text(String),
    ToolCalls(Vec<OllamaToolCall>),
}

#[derive(Debug, Clone)]
pub struct OllamaClient {
    client: Client,
    base_url: String,
    model: String,
}

impl OllamaClient {
    pub fn new() -> Self {
        let base_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| OLLAMA_DEFAULT_URL.to_string());
        let model = std::env::var("OLLAMA_MODEL")
            .unwrap_or_else(|_| OLLAMA_DEFAULT_MODEL.to_string());
        Self {
            client: Client::new(),
            base_url,
            model,
        }
    }

    /// Text-only generation (simple string prompt).
    pub async fn generate_text(
        &self,
        prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": 2048,
            "temperature": 0.1,
            "stream": false,
        });

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(format!("Ollama error {}: {}", status, err).into());
        }

        let json: serde_json::Value = resp.json().await?;
        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or("Ollama: no content in response")?
            .to_string();
        Ok(text)
    }

    /// Multimodal generation: sends text + base64-encoded images (JPEG) in
    /// the OpenAI-compatible content parts format.
    ///
    /// `images` is a vec of (label, jpeg_bytes) pairs. Each image is sent as
    /// an `image_url` part with `data:image/jpeg;base64,...`.
    pub async fn generate_text_with_images(
        &self,
        prompt: &str,
        images: Vec<(String, Vec<u8>)>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut content_parts: Vec<serde_json::Value> = Vec::new();

        for (label, bytes) in &images {
            if !label.is_empty() {
                content_parts.push(serde_json::json!({
                    "type": "text",
                    "text": label
                }));
            }
            let b64 = BASE64_STANDARD.encode(bytes);
            content_parts.push(serde_json::json!({
                "type": "image_url",
                "image_url": {
                    "url": format!("data:image/jpeg;base64,{}", b64)
                }
            }));
        }

        content_parts.push(serde_json::json!({
            "type": "text",
            "text": prompt
        }));

        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": content_parts}],
            "max_tokens": 8192,
            "temperature": 0.3,
            "stream": false,
        });

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(format!("Ollama vision error {}: {}", status, err).into());
        }

        let json: serde_json::Value = resp.json().await?;
        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or("Ollama: no content in multimodal response")?
            .to_string();
        Ok(text)
    }

    // ─── Tool calling (OpenAI-compatible, same as DeepSeek) ─────────────────

    fn to_openai_tools(decls: &[crate::gemini_client::FunctionDeclaration]) -> serde_json::Value {
        let tools: Vec<serde_json::Value> = decls
            .iter()
            .map(|d| {
                let props: serde_json::Map<String, serde_json::Value> = d
                    .parameters
                    .properties
                    .iter()
                    .map(|(k, v)| {
                        let mut prop = serde_json::json!({
                            "type": v.prop_type,
                            "description": v.description,
                        });
                        if let Some(ref items) = v.items {
                            prop["items"] = serde_json::json!({ "type": items });
                        }
                        (k.clone(), prop)
                    })
                    .collect();

                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": d.name,
                        "description": d.description,
                        "parameters": {
                            "type": "object",
                            "properties": props,
                            "required": d.parameters.required,
                        }
                    }
                })
            })
            .collect();

        serde_json::Value::Array(tools)
    }

    pub async fn generate_single(
        &self,
        messages: &[serde_json::Value],
        tools: &[crate::gemini_client::FunctionDeclaration],
    ) -> Result<OllamaResponse, Box<dyn std::error::Error + Send + Sync>> {
        let openai_tools = Self::to_openai_tools(tools);

        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": openai_tools,
            "tool_choice": "auto",
            "max_tokens": 8192,
            "temperature": 0.5,
        });

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(format!("Ollama tool call error {}: {}", status, err).into());
        }

        let json: serde_json::Value = resp.json().await?;
        let choice = &json["choices"][0];
        let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

        if finish_reason == "tool_calls" {
            let tool_calls: Vec<OllamaToolCall> = choice["message"]["tool_calls"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|tc| {
                    let id = tc["id"].as_str()?.to_string();
                    let name = tc["function"]["name"].as_str()?.to_string();
                    let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                    let arguments =
                        serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
                    Some(OllamaToolCall { id, name, arguments })
                })
                .collect();
            return Ok(OllamaResponse::ToolCalls(tool_calls));
        }

        let text = choice["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(OllamaResponse::Text(text))
    }

    /// Analyze a locally downloaded video file by extracting JPEG frames and
    /// sending them to Gemma 4 12B as a multimodal request.
    ///
    /// Mirrors `GeminiClient::analyze_video_from_local_file` but uses Ollama's
    /// vision capability instead of Gemini. Produces the same `VideoAnalysis`
    /// schema so callers are interchangeable.
    ///
    /// Frame count: 1 frame per 2 minutes of footage, clamped 8–20.
    pub async fn analyze_video_from_local_file(
        &self,
        video_path: &str,
        clips_per_video: usize,
        min_duration_secs: f64,
        max_duration_secs: f64,
        high_performing_factors: &[String],
    ) -> Result<
        crate::clipping::gemini_video_analyzer::VideoAnalysis,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        tracing::info!(
            "🎬 Ollama: analyzing local video via frames — {}",
            video_path
        );

        // Get total duration via ffprobe
        let total_dur = crate::core::get_video_duration(video_path)
            .map_err(|e| format!("Ollama: failed to get video duration: {}", e))?;

        // 1 frame per 2 minutes, clamped 8–20
        let num_frames = ((total_dur / 120.0).round() as usize).clamp(8, 20);

        // Extract frames at evenly spaced timestamps
        let mut frame_paths: Vec<String> = Vec::new();
        let mut frame_timestamps: Vec<f64> = Vec::new();
        for i in 0..num_frames {
            let ts = if num_frames == 1 {
                total_dur * 0.5
            } else {
                total_dur * (i as f64 / (num_frames - 1) as f64)
            };
            let ts = ts.clamp(0.1, total_dur - 0.1);
            let path = crate::utils::ffmpeg_utils::create_temp_file(
                &format!("ollama_frame_{}", i),
                "jpg",
            );
            match crate::utils::ffmpeg_utils::extract_frame_at_timestamp(video_path, ts, &path) {
                Ok(p) => {
                    frame_paths.push(p);
                    frame_timestamps.push(ts);
                }
                Err(e) => {
                    tracing::warn!(
                        "Ollama: frame {}/{} at {:.1}s failed (skipping): {}",
                        i + 1,
                        num_frames,
                        ts,
                        e
                    );
                }
            }
        }

        if frame_paths.is_empty() {
            return Err(
                format!("Ollama: failed to extract any frames from '{}'", video_path).into(),
            );
        }

        // Read frame bytes then clean up temp files
        let mut frame_data: Vec<(f64, Vec<u8>)> = Vec::new();
        for (path, ts) in frame_paths.iter().zip(frame_timestamps.iter()) {
            match tokio::fs::read(path).await {
                Ok(bytes) => frame_data.push((*ts, bytes)),
                Err(e) => tracing::warn!("Ollama: failed to read frame {}: {}", path, e),
            }
        }
        crate::utils::ffmpeg_utils::cleanup_temp_files(&frame_paths);

        if frame_data.is_empty() {
            return Err("Ollama: failed to read any frame data".into());
        }

        tracing::info!(
            "📸 Ollama: extracted {}/{} frames for analysis (video: {:.0}s)",
            frame_data.len(),
            num_frames,
            total_dur
        );

        let learned_factors_hint = if !high_performing_factors.is_empty() {
            format!(
                "\nLEARNED HIGH-PERFORMING FACTORS (prioritize moments containing these): {}\n",
                high_performing_factors.join(", ")
            )
        } else {
            String::new()
        };

        let prompt = format!(
            r#"You are analyzing a video via {n} sampled frames. Total duration: {total_dur:.0}s ({total_min:.1} minutes).

Each frame is labeled with its exact timestamp [t=Xs]. Use these timestamps to estimate accurate start/end times for viral clips.

Identify exactly {clips_per_video} viral clip opportunities for YouTube Shorts.

REQUIREMENTS:
- Each clip must be between {min_dur:.0}s and {max_dur:.0}s (HARD LIMIT — never exceed {max_dur:.0}s)
- Clips will be published as YouTube Shorts (vertical 9:16 portrait format, center-cropped from landscape)
- Prioritize moments where the subject is centered in frame
- Focus on: dramatic hooks, surprising moments, emotional peaks, action sequences, plot twists
- Clips should work as standalone content without needing context{learned_hint}

Return ONLY a valid JSON object matching this exact schema:
{{
  "video_summary": "<comprehensive plain-text summary of the entire video>",
  "content_type": "<one of: entertainment, tutorial, news, gaming, sports, music, vlog, other>",
  "overall_quality": <float 0.0-1.0>,
  "viral_moments": [
    {{
      "start_sec": <float seconds>,
      "end_sec": <float seconds>,
      "title": "<engaging YouTube Short title, max 60 chars>",
      "hook": "<first sentence that grabs attention>",
      "quality_score": <float 0.0-1.0>,
      "viral_factors": ["<factor1>", "<factor2>"],
      "thumbnail_sec": <float seconds — best frame for thumbnail within the clip>,
      "reason": "<why this moment is viral/engaging>"
    }}
  ]
}}

Provide ONLY the JSON object, no markdown, no code blocks, no other text."#,
            n = frame_data.len(),
            total_dur = total_dur,
            total_min = total_dur / 60.0,
            clips_per_video = clips_per_video,
            min_dur = min_duration_secs,
            max_dur = max_duration_secs,
            learned_hint = learned_factors_hint,
        );

        // Build images array: each image gets a text label with timestamp
        let mut images: Vec<(String, Vec<u8>)> = Vec::new();
        for (ts, bytes) in frame_data.iter() {
            images.push((format!("[t={:.1}s]", ts), bytes.clone()));
        }

        let raw = self
            .generate_text_with_images(&prompt, images)
            .await?;

        // Clean markdown code fences if present
        let cleaned = raw
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let analysis: crate::clipping::gemini_video_analyzer::VideoAnalysis =
            serde_json::from_str(cleaned).map_err(|e| {
                format!(
                    "Ollama: failed to parse VideoAnalysis JSON: {} — text: {}",
                    e,
                    &cleaned[..cleaned.len().min(500)]
                )
            })?;

        tracing::info!(
            "✅ Ollama analysis complete: {} viral moments (quality: {:.2})",
            analysis.viral_moments.len(),
            analysis.overall_quality
        );

        Ok(analysis)
    }

    /// Analyze a YouTube (or any) video URL by downloading it first via Apify,
    /// then running local frame analysis through Ollama.
    ///
    /// Mirrors the Twitch fallback path in Gemini — Ollama (and Gemma 4) cannot
    /// directly fetch YouTube URLs, so we download and analyze frames locally.
    pub async fn analyze_video_from_url(
        &self,
        video_url: &str,
        clips_per_video: usize,
        min_duration_secs: f64,
        max_duration_secs: f64,
        high_performing_factors: &[String],
    ) -> Result<
        crate::clipping::gemini_video_analyzer::VideoAnalysis,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        tracing::info!("🎬 Ollama: analyzing video URL via download+frames — {}", video_url);

        // Create a temporary download path
        let dl_path = crate::utils::ffmpeg_utils::create_temp_file("ollama_url_analysis", "mp4");

        // Download via Apify
        let apify_token =
            std::env::var("APIFY_TOKEN").map_err(|_| "APIFY_TOKEN not configured".to_string())?;
        let apify_actor = std::env::var("APIFY_YOUTUBE_CLIENT_ACTOR")
            .map_err(|_| "APIFY_YOUTUBE_CLIENT_ACTOR not configured".to_string())?;
        let apify_client =
            crate::clipping::apify_client::ApifyClient::new(apify_token, apify_actor);

        tracing::info!("⬇️ Ollama: downloading {} for frame analysis", video_url);
        apify_client
            .download_video(video_url, &dl_path)
            .await
            .map_err(|e| {
                format!(
                    "Ollama: failed to download video for analysis: {}",
                    e
                )
            })?;

        // Validate download
        if !std::path::Path::new(&dl_path).exists() {
            return Err(format!("Ollama: downloaded file not found: {}", dl_path).into());
        }
        match crate::core::validate_video_file(&dl_path) {
            Ok(true) => {}
            _ => {
                let _ = tokio::fs::remove_file(&dl_path).await;
                return Err(format!("Ollama: downloaded video is corrupted: {}", dl_path).into());
            }
        }

        // Analyze the downloaded local file
        let result = self
            .analyze_video_from_local_file(
                &dl_path,
                clips_per_video,
                min_duration_secs,
                max_duration_secs,
                high_performing_factors,
            )
            .await;

        // Cleanup temp download
        let _ = tokio::fs::remove_file(&dl_path).await;

        result
    }
}
