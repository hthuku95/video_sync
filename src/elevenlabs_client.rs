// Eleven Labs API Client
// Supports: Text-to-Speech, Sound Effects, Music Generation

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone)]
pub struct ElevenLabsClient {
    api_key: String,
    client: Client,
    base_url: String,
}

// ============================================================================
// API REQUEST/RESPONSE STRUCTURES
// ============================================================================

#[derive(Serialize, Debug)]
pub struct TextToSpeechRequest {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_settings: Option<VoiceSettings>,
}

#[derive(Serialize, Debug)]
pub struct VoiceSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stability: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity_boost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_speaker_boost: Option<bool>,
}

#[derive(Serialize, Debug)]
pub struct SoundEffectsRequest {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_influence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct MusicGenerationRequest {
    pub prompt: String,
    pub duration: u32, // milliseconds (10000-300000)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct MusicGenerationResponse {
    pub generation_id: String,
}

#[derive(Deserialize, Debug)]
pub struct MusicStatusResponse {
    pub status: String, // "pending", "completed", "failed"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct Voice {
    pub voice_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct VoicesResponse {
    pub voices: Vec<Voice>,
}

// ============================================================================
// IMPLEMENTATION
// ============================================================================

impl ElevenLabsClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: Client::new(),
            base_url: "https://api.elevenlabs.io/v1".to_string(),
        }
    }

    /// Generate speech from text using a specific voice
    pub async fn text_to_speech(
        &self,
        text: &str,
        voice_id: &str,
        model_id: Option<&str>,
        voice_settings: Option<VoiceSettings>,
        output_format: Option<&str>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/text-to-speech/{}", self.base_url, voice_id);

        let request_body = TextToSpeechRequest {
            text: text.to_string(),
            model_id: model_id.map(|s| s.to_string()),
            language_code: None,
            voice_settings,
        };

        let mut request = self.client
            .post(&url)
            .header("xi-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&request_body);

        // Add output format as query parameter if specified
        if let Some(format) = output_format {
            request = request.query(&[("output_format", format)]);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("Eleven Labs TTS API error ({}): {}", status, error_text).into());
        }

        let audio_bytes = response.bytes().await?;
        Ok(audio_bytes.to_vec())
    }

    /// Generate sound effects from text description
    pub async fn generate_sound_effect(
        &self,
        description: &str,
        duration_seconds: Option<f64>,
        prompt_influence: Option<f64>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/sound-generation", self.base_url);

        let request_body = SoundEffectsRequest {
            text: description.to_string(),
            duration_seconds,
            prompt_influence,
            model_id: Some("eleven_text_to_sound_v2".to_string()),
        };

        let response = self.client
            .post(&url)
            .header("xi-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("Eleven Labs Sound Effects API error ({}): {}", status, error_text).into());
        }

        let audio_bytes = response.bytes().await?;
        Ok(audio_bytes.to_vec())
    }

    /// Generate music from text prompt (Step 1: Create task)
    pub async fn generate_music_task(
        &self,
        prompt: &str,
        duration_ms: u32,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/music-generation", self.base_url);

        let request_body = MusicGenerationRequest {
            prompt: prompt.to_string(),
            duration: duration_ms,
            model_id: Some("eleven_music_v1".to_string()),
        };

        let response = self.client
            .post(&url)
            .header("xi-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("Eleven Labs Music API error ({}): {}", status, error_text).into());
        }

        let response_data: MusicGenerationResponse = response.json().await?;
        Ok(response_data.generation_id)
    }

    /// Check music generation status (Step 2: Poll for result)
    pub async fn get_music_status(
        &self,
        generation_id: &str,
    ) -> Result<MusicStatusResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/music-generation/{}", self.base_url, generation_id);

        let response = self.client
            .get(&url)
            .header("xi-api-key", &self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("Eleven Labs Music Status API error ({}): {}", status, error_text).into());
        }

        let status_data: MusicStatusResponse = response.json().await?;
        Ok(status_data)
    }

    /// Download generated music audio
    pub async fn download_music(
        &self,
        audio_url: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let response = self.client
            .get(audio_url)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err("Failed to download music audio".into());
        }

        let audio_bytes = response.bytes().await?;
        Ok(audio_bytes.to_vec())
    }

    /// List all available voices
    pub async fn list_voices(&self) -> Result<Vec<Voice>, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/voices", self.base_url);

        let response = self.client
            .get(&url)
            .header("xi-api-key", &self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("Eleven Labs Voices API error ({}): {}", status, error_text).into());
        }

        let voices_data: VoicesResponse = response.json().await?;
        Ok(voices_data.voices)
    }

    /// Get a specific voice by ID
    pub async fn get_voice(&self, voice_id: &str) -> Result<Voice, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/voices/{}", self.base_url, voice_id);

        let response = self.client
            .get(&url)
            .header("xi-api-key", &self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("Eleven Labs Get Voice API error ({}): {}", status, error_text).into());
        }

        let voice: Voice = response.json().await?;
        Ok(voice)
    }
}

// ============================================================================
// WELL-KNOWN VOICE IDS (Default Voices)
// ============================================================================

pub struct DefaultVoices;

impl DefaultVoices {
    // Female voices
    pub const RACHEL: &'static str = "21m00Tcm4TlvDq8ikWAM"; // Young female, calm
    pub const DOMI: &'static str = "AZnzlk1XvdvUeBnXmlld"; // Female, strong
    pub const BELLA: &'static str = "EXAVITQu4vr4xnSDxMaL"; // Female, soft
    pub const ELLI: &'static str = "MF3mGyEYCl7XYWbV9V6O"; // Female, emotional
    pub const EMILY: &'static str = "LcfcDJNUP1GQjkzn1xUU"; // Female, calm
    pub const GRACE: &'static str = "oWAxZDx7w5VEj9dCyTzz"; // Female, young
    pub const MATILDA: &'static str = "XrExE9yKIg1WjnnlVkGX"; // Female, warm

    // Male voices
    pub const DREW: &'static str = "29vD33N1CtxCmqQRPOHJ"; // Male, middle-aged
    pub const CLYDE: &'static str = "2EiwWnXFnvU5JabPnv8n"; // Male, war veteran
    pub const PAUL: &'static str = "5Q0t7uMcjvnagumLfvZi"; // Male, ground reporter
    pub const ADAM: &'static str = "pNInz6obpgDQGcFmaJgB"; // Male, deep
    pub const ARNOLD: &'static str = "VR6AewLTigWG4xSOukaG"; // Male, crisp
    pub const CALLUM: &'static str = "N2lVS1w4EtoT3dr4eOWO"; // Male, hoarse
    pub const DANIEL: &'static str = "onwK4e9ZLuTAKqWW03F9"; // Male, deep
    pub const ETHAN: &'static str = "g5CIjZEefAph4nQFvHAz"; // Male, young
    pub const LIAM: &'static str = "TX3LPaxmHKxFdv7VOQHJ"; // Male, articulate
    pub const THOMAS: &'static str = "GBv7mTt0atIp3Br8iCZE"; // Male, calm

    pub fn get_voice_name(voice_id: &str) -> &'static str {
        match voice_id {
            Self::RACHEL => "Rachel (Female, Young, Calm)",
            Self::DOMI => "Domi (Female, Strong)",
            Self::BELLA => "Bella (Female, Soft)",
            Self::ELLI => "Elli (Female, Emotional)",
            Self::EMILY => "Emily (Female, Calm)",
            Self::GRACE => "Grace (Female, Young)",
            Self::MATILDA => "Matilda (Female, Warm)",
            Self::DREW => "Drew (Male, Middle-aged)",
            Self::CLYDE => "Clyde (Male, War Veteran)",
            Self::PAUL => "Paul (Male, Reporter)",
            Self::ADAM => "Adam (Male, Deep)",
            Self::ARNOLD => "Arnold (Male, Crisp)",
            Self::CALLUM => "Callum (Male, Hoarse)",
            Self::DANIEL => "Daniel (Male, Deep)",
            Self::ETHAN => "Ethan (Male, Young)",
            Self::LIAM => "Liam (Male, Articulate)",
            Self::THOMAS => "Thomas (Male, Calm)",
            _ => "Unknown Voice",
        }
    }

    pub fn get_voice_id_by_name(name: &str) -> Option<&'static str> {
        match name.to_lowercase().as_str() {
            "rachel" => Some(Self::RACHEL),
            "domi" => Some(Self::DOMI),
            "bella" => Some(Self::BELLA),
            "elli" => Some(Self::ELLI),
            "emily" => Some(Self::EMILY),
            "grace" => Some(Self::GRACE),
            "matilda" => Some(Self::MATILDA),
            "drew" => Some(Self::DREW),
            "clyde" => Some(Self::CLYDE),
            "paul" => Some(Self::PAUL),
            "adam" => Some(Self::ADAM),
            "arnold" => Some(Self::ARNOLD),
            "callum" => Some(Self::CALLUM),
            "daniel" => Some(Self::DANIEL),
            "ethan" => Some(Self::ETHAN),
            "liam" => Some(Self::LIAM),
            "thomas" => Some(Self::THOMAS),
            _ => None,
        }
    }

    pub fn get_default_female_voice() -> &'static str {
        Self::RACHEL
    }

    pub fn get_default_male_voice() -> &'static str {
        Self::DREW
    }

    pub fn get_default_voice() -> &'static str {
        Self::RACHEL
    }
}

// ============================================================================
// MODELS
// ============================================================================

pub struct ElevenLabsModels;

impl ElevenLabsModels {
    pub const FLASH_V2_5: &'static str = "eleven_flash_v2_5"; // 75ms latency - ultra-fast
    pub const MULTILINGUAL_V2: &'static str = "eleven_multilingual_v2"; // Highest quality
    pub const TURBO_V2_5: &'static str = "eleven_turbo_v2_5"; // Fast with good quality
    pub const MONOLINGUAL_V1: &'static str = "eleven_monolingual_v1"; // English only
}
