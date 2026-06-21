/// Ollama client — self-hosted LLM via Ollama OpenAI-compatible API.
/// Default model: gemma4:12b (encoder-free multimodal, vision+audio, ~7.6GB).
/// Configurable via OLLAMA_BASE_URL and OLLAMA_MODEL env vars.
///
/// Vision support: uses OpenAI-compatible content parts array with
/// `image_url` parts (base64-encoded JPEG) for multimodal analysis.
use base64::prelude::*;
use reqwest::Client;

pub const OLLAMA_DEFAULT_URL: &str = "http://172.31.43.45:11434";
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
    /// Return the configured model ID.
    pub fn model_id(&self) -> &str {
        &self.model
    }

    /// Send a raw JSON body to Ollama's native `/api/chat` endpoint.
    /// Used for native video analysis (Gemma 4) where the full video is sent
    /// as base64 in the `images` field.
    pub async fn chat_native(
        &self,
        body: serde_json::Value,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Ollama native chat request failed: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(format!("Ollama native chat error {}: {}", status, err_body).into());
        }

        let json: serde_json::Value = resp.json().await
            .map_err(|e| format!("Ollama native chat parse error: {}", e))?;

        json["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "Ollama native chat: no content in response".into())
    }

    pub fn new() -> Self {
        let base_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| OLLAMA_DEFAULT_URL.to_string());
        let model = std::env::var("OLLAMA_MODEL")
            .unwrap_or_else(|_| OLLAMA_DEFAULT_MODEL.to_string());
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(900))
                .connect_timeout(std::time::Duration::from_secs(10))
                .pool_max_idle_per_host(0)
                .build()
                .unwrap_or_default(),
            base_url,
            model,
        }
    }

    /// Warm up the Ollama model by sending a lightweight chat request.
    /// This forces the model to load into memory so subsequent calls are fast.
    /// Should be called once at server startup. Non-fatal on failure.
    pub async fn warmup(&self) {
        tracing::info!("Ollama warmup: loading model '{}' from {}...", self.model, self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": "Hello — respond with exactly 'OK'."}],
            "think": false,
            "options": {"num_predict": 10, "temperature": 0.0},
            "stream": false,
        });
        match tokio::time::timeout(
            std::time::Duration::from_secs(180),
            self.client
                .post(format!("{}/api/chat", self.base_url))
                .header("Content-Type", "application/json")
                .json(&body)
                .send(),
        )
        .await
        {
            Ok(Ok(resp)) => {
                let status = resp.status();
                if status.is_success() {
                    tracing::info!("Ollama warmup complete — model '{}' is loaded", self.model);
                } else {
                    let err_body = resp.text().await.unwrap_or_default();
                    tracing::warn!("Ollama warmup returned {} (non-fatal): {}", status, err_body);
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("Ollama warmup request failed (non-fatal): {}", e);
            }
            Err(_) => {
                tracing::warn!("Ollama warmup timed out after 180s (non-fatal)");
            }
        }
    }

    /// Text-only generation (simple string prompt).
    pub async fn generate_text(
        &self,
        prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": "You are a helpful assistant. Respond directly without thinking or reasoning. Never use reasoning tags."},
                {"role": "user", "content": prompt}
            ],
            "temperature": 0.1,
            "think": false,
            "options": {"num_predict": 2048},
            "stream": false,
        });

        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
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
        let text = json["message"]["content"]
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
            "temperature": 0.3,
            "think": false,
            "options": {"num_predict": 4096},
            "stream": false,
        });

        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
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
        let text = json["message"]["content"]
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
            "think": false,
            "stream": false,
            "options": {
                "num_predict": 1024,
                "temperature": 0.3,
            },
        });

        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(format!("Ollama API error {}: {}", status, err).into());
        }

        let json: serde_json::Value = resp.json().await?;
        let msg = &json["message"];

        // Check for tool calls first
        if let Some(tool_calls) = msg["tool_calls"].as_array() {
            if !tool_calls.is_empty() {
                let calls: Vec<OllamaToolCall> = tool_calls
                    .iter()
                    .filter_map(|tc| {
                        let id = tc["id"].as_str()?.to_string();
                        let name = tc["function"]["name"].as_str()?.to_string();
                        let arguments = tc["function"]["arguments"].clone();
                        Some(OllamaToolCall { id, name, arguments })
                    })
                    .collect();
                if !calls.is_empty() {
                    return Ok(OllamaResponse::ToolCalls(calls));
                }
            }
        }

        let text = msg["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(OllamaResponse::Text(text))
    }

    /// Analyze a video by sending the FULL video bytes to Gemma 4 12B's native
    /// multimodal API. Gemma 4 natively understands video including motion, audio,
    /// pacing, and timing across the full timeline — this sends the entire video
    /// file, NOT extracted frames.
    ///
    /// Uses Ollama's native `/api/chat` endpoint with the complete video file as
    /// base64-encoded binary data. The model handles internal frame processing
    /// and temporal reasoning natively.
    ///
    /// Produces the same `VideoAnalysis` schema as `GeminiClient::analyze_video_from_url`.
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
            "Ollama: analyzing video via native full-video API — {}",
            video_path
        );

        // Read the full video file bytes
        let video_bytes = tokio::fs::read(video_path)
            .await
            .map_err(|e| format!("Ollama: failed to read video '{}': {}", video_path, e))?;

        if video_bytes.is_empty() {
            return Err(format!("Ollama: video '{}' is empty", video_path).into());
        }

        let total_len_mb = video_bytes.len() as f64 / 1_048_576.0;

        // Get total duration via ffprobe
        let total_dur = crate::core::get_video_duration(video_path)
            .map_err(|e| format!("Ollama: failed to get video duration: {}", e))?;

        let learned_factors_hint = if !high_performing_factors.is_empty() {
            format!(
                "\nLEARNED HIGH-PERFORMING FACTORS (prioritize moments containing these): {}\n",
                high_performing_factors.join(", ")
            )
        } else {
            String::new()
        };

        let prompt = format!(
            r#"Analyze this video natively — Gemma 4 sees the FULL video with motion, audio, pacing, and timing.

Total duration: {total_dur:.0}s ({total_min:.1} minutes). File size: {total_len_mb:.1}MB.

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
            total_dur = total_dur,
            total_min = total_dur / 60.0,
            total_len_mb = total_len_mb,
            clips_per_video = clips_per_video,
            min_dur = min_duration_secs,
            max_dur = max_duration_secs,
            learned_hint = learned_factors_hint,
        );

        // Send the full video to Ollama's native api/chat endpoint.
        // The video bytes are base64-encoded and sent as the sole media element.
        // Gemma 4 natively processes the full video internally.
        let b64 = BASE64_STANDARD.encode(&video_bytes);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": prompt,
                "images": [b64]
            }],
            "think": false,
            "stream": false,
            "options": {
                "num_predict": 8192,
                "temperature": 0.3
            }
        });

        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Ollama video analysis request failed: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(format!("Ollama video analysis error {}: {}", status, err_body).into());
        }

        let json: serde_json::Value = resp.json().await
            .map_err(|e| format!("Ollama video analysis parse error: {}", e))?;

        let raw = json["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        if raw.is_empty() {
            return Err("Ollama video analysis returned empty response".into());
        }

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
            "Ollama native video analysis complete: {} viral moments (quality: {:.2})",
            analysis.viral_moments.len(),
            analysis.overall_quality
        );

        Ok(analysis)
    }

    /// Analyze a YouTube (or any) video URL by downloading it first via Apify,
    /// then sending the full video to Ollama/Gemma 4 for native video analysis.
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
        tracing::info!("Ollama: analyzing video URL — {}", video_url);

        // Create a temporary download path
        let dl_path = crate::utils::ffmpeg_utils::create_temp_file("ollama_url_analysis", "mp4");

        // Download via Apify
        let apify_token =
            std::env::var("APIFY_TOKEN").map_err(|_| "APIFY_TOKEN not configured".to_string())?;
        let apify_actor = std::env::var("APIFY_YOUTUBE_CLIENT_ACTOR")
            .map_err(|_| "APIFY_YOUTUBE_CLIENT_ACTOR not configured".to_string())?;
        let apify_client =
            crate::clipping::apify_client::ApifyClient::new(apify_token, apify_actor);

        tracing::info!("Downloading {} for Ollama video analysis", video_url);
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
