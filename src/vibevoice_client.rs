use reqwest::Client;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// RunPod serverless v2 base URL prefix. When `VIBEVOICE_SERVICE_URL` starts
/// with this, the client speaks the RunPod job queue protocol instead of the
/// legacy FastAPI microservice contract (infra/media_ai + VibeVoiceService).
const RUNPOD_BASE_PREFIX: &str = "https://api.runpod.ai/v2/";

/// Poll interval for RunPod job status.
const RUNPOD_POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Hard deadline for a RunPod job (client HTTP timeout is 300s per request).
const RUNPOD_JOB_DEADLINE: Duration = Duration::from_secs(280);

#[derive(Clone)]
pub struct VibeVoiceClient {
    client: Client,
    pub base_url: String,
    api_key: Option<String>,
}

impl VibeVoiceClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_default();

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.filter(|key| !key.trim().is_empty()),
        }
    }

    fn with_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(api_key) = &self.api_key {
            req.header("Authorization", format!("Bearer {}", api_key))
        } else {
            req
        }
    }

    fn is_runpod(&self) -> bool {
        self.base_url.starts_with(RUNPOD_BASE_PREFIX)
    }

    /// Submit a job to the RunPod serverless queue and poll until completion.
    ///
    /// `input` carries `kind` ("tts" | "transcribe") plus the legacy contract
    /// fields; the handler contract is documented in
    /// `VideoSyncIntegrations/VibeVoiceRunpod/handler.py`.
    async fn runpod_run_job(&self, input: Value, label: &str) -> Result<Value, String> {
        let run_url = format!("{}/run", self.base_url);
        let resp = self
            .with_auth(self.client.post(&run_url).json(&json!({ "input": input })))
            .send()
            .await
            .map_err(|e| format!("VibeVoice RunPod submit ({label}) failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("VibeVoice RunPod submit ({label}) error {status}: {body}"));
        }

        let submitted: Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse RunPod submit response ({label}): {e}"))?;

        let job_id = submitted
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("RunPod submit response missing job id ({label})"))?
            .to_string();

        let deadline = Instant::now() + RUNPOD_JOB_DEADLINE;
        let status_url = format!("{}/status/{}", self.base_url, job_id);

        loop {
            tokio::time::sleep(RUNPOD_POLL_INTERVAL).await;
            if Instant::now() >= deadline {
                return Err(format!(
                    "VibeVoice RunPod job {job_id} ({label}) timed out after {:?}",
                    RUNPOD_JOB_DEADLINE
                ));
            }

            let status_resp = self
                .with_auth(self.client.get(&status_url))
                .send()
                .await
                .map_err(|e| format!("VibeVoice RunPod status request failed: {e}"))?;

            if status_resp.status() == reqwest::StatusCode::NOT_FOUND {
                // Result may have been purged/expired already; keep polling briefly.
                continue;
            }

            if !status_resp.status().is_success() {
                let status = status_resp.status();
                let body = status_resp.text().await.unwrap_or_default();
                return Err(format!("VibeVoice RunPod status error {status}: {body}"));
            }

            let job: Value = status_resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse RunPod status response: {e}"))?;

            let state = job
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN")
                .to_uppercase();

            match state.as_str() {
                "COMPLETED" => {
                    return job
                        .get("output")
                        .filter(|v| v.is_object())
                        .cloned()
                        .ok_or_else(|| format!("RunPod job {job_id} completed without output"));
                }
                "FAILED" | "TIMED_OUT" | "CANCELLED" => {
                    let detail = job
                        .get("error")
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| format!("status {state}"));
                    return Err(format!("VibeVoice RunPod job ({label}) failed: {detail}"));
                }
                _ => continue,
            }
        }
    }

    pub async fn health(&self) -> Result<Value, String> {
        let url = format!("{}/health", self.base_url);
        let resp = self
            .with_auth(self.client.get(&url))
            .send()
            .await
            .map_err(|e| format!("VibeVoice health request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("VibeVoice health error {status}: {body}"));
        }

        resp.json()
            .await
            .map_err(|e| format!("Failed to parse VibeVoice health response: {e}"))
    }

    pub async fn text_to_speech_base64(
        &self,
        text: &str,
        speaker: &str,
        format: &str,
        job_id: Option<&str>,
        metadata: Option<Value>,
    ) -> Result<Vec<u8>, String> {
        let input = json!({
            "text": text,
            "speaker": speaker,
            "format": format,
            "job_id": job_id,
            "metadata": metadata.unwrap_or_else(|| json!({})),
        });

        let value = if self.is_runpod() {
            let mut queue_input = input.clone();
            queue_input["kind"] = json!("tts");
            self.runpod_run_job(queue_input, "tts").await?
        } else {
            let url = format!("{}/api/tts/base64", self.base_url);

            let resp = self
                .with_auth(self.client.post(&url).json(&input))
                .send()
                .await
                .map_err(|e| format!("VibeVoice TTS request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("VibeVoice TTS error {status}: {body}"));
            }

            resp.json()
                .await
                .map_err(|e| format!("Failed to parse VibeVoice TTS response: {e}"))?
        };

        let base64_audio = value
            .get("audio_base64")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "VibeVoice response missing audio_base64".to_string())?;

        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, base64_audio)
            .map_err(|e| format!("Failed to decode VibeVoice base64 audio: {e}"))
    }

    pub async fn transcribe_url(
        &self,
        audio_url: &str,
        hotwords: &[String],
        language: Option<&str>,
        job_id: Option<&str>,
        metadata: Option<Value>,
    ) -> Result<Value, String> {
        let input = json!({
            "audio_url": audio_url,
            "hotwords": hotwords,
            "language": language,
            "job_id": job_id,
            "metadata": metadata.unwrap_or_else(|| json!({})),
        });

        if self.is_runpod() {
            let mut queue_input = input.clone();
            queue_input["kind"] = json!("transcribe");
            return self.runpod_run_job(queue_input, "transcribe").await;
        }

        let url = format!("{}/api/transcribe", self.base_url);

        let resp = self
            .with_auth(self.client.post(&url).json(&input))
            .send()
            .await
            .map_err(|e| format!("VibeVoice transcription request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("VibeVoice transcription error {status}: {body}"));
        }

        resp.json()
            .await
            .map_err(|e| format!("Failed to parse VibeVoice transcription response: {e}"))
    }
}
