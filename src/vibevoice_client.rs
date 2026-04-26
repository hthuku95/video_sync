use reqwest::Client;
use serde_json::{json, Value};

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
        let url = format!("{}/api/tts/base64", self.base_url);
        let body = json!({
            "text": text,
            "speaker": speaker,
            "format": format,
            "job_id": job_id,
            "metadata": metadata.unwrap_or_else(|| json!({})),
        });

        let resp = self
            .with_auth(self.client.post(&url).json(&body))
            .send()
            .await
            .map_err(|e| format!("VibeVoice TTS request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("VibeVoice TTS error {status}: {body}"));
        }

        let payload: Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse VibeVoice TTS response: {e}"))?;

        let base64_audio = payload
            .get("audio_base64")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "VibeVoice response missing audio_base64".to_string())?;

        base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            base64_audio,
        )
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
        let url = format!("{}/api/transcribe", self.base_url);
        let body = json!({
            "audio_url": audio_url,
            "hotwords": hotwords,
            "language": language,
            "job_id": job_id,
            "metadata": metadata.unwrap_or_else(|| json!({})),
        });

        let resp = self
            .with_auth(self.client.post(&url).json(&body))
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
