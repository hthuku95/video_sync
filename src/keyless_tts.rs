//! Keyless HTTP TTS — the fourth narration provider.
//!
//! Production runs three TTS providers and, as of Nov 2026, all three failed at
//! once: ElevenLabs returned 402 (balance), the VibeVoice CPU worker queued so
//! slowly that requests hit its 240 s queue timeout, and Gemini returned 403
//! (billing). Without a fourth path every render dies at
//! `ARTIFACT_INVALID: NO audio stream`.
//!
//! This module talks to Google Translate's public `translate_tts` endpoint: no
//! API key, no quota account, ~0.8 s for a phrase. It is deliberately a
//! last-resort (or opt-in first) provider — the endpoint is unofficial and has
//! no SLA, so it must never shadow a healthy paid provider unless the operator
//! asks for it with `KEYLESS_TTS_FIRST=1`.
//!
//! Controls:
//!   KEYLESS_TTS_ENABLED  "0" disables the fallback entirely (default: on)
//!   KEYLESS_TTS_FIRST    "1"/"true" tries it before VibeVoice (default: off)
//!   KEYLESS_TTS_LANG     language code override (default: "en")
//!   KEYLESS_TTS_SPEED    "0.24".."1" playback speed (default: "1")

use std::time::Duration;

/// Google's endpoint rejects long strings; 180 chars is comfortably safe.
const MAX_CHUNK_CHARS: usize = 180;
const REQUEST_TIMEOUT_SECS: u64 = 25;
const ENDPOINT: &str = "https://translate.google.com/translate_tts";
const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

pub fn enabled() -> bool {
    !matches!(
        std::env::var("KEYLESS_TTS_ENABLED").as_deref(),
        Ok("0") | Ok("false") | Ok("no")
    )
}

pub fn first() -> bool {
    matches!(
        std::env::var("KEYLESS_TTS_FIRST").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Map the tool's voice names onto a language code. Google's endpoint selects a
/// voice from the language, not by name.
pub fn lang_for_voice(voice: &str) -> String {
    if let Ok(explicit) = std::env::var("KEYLESS_TTS_LANG") {
        if !explicit.trim().is_empty() {
            return explicit.trim().to_string();
        }
    }
    let v = voice.to_ascii_lowercase();
    if v.contains("british") || v.contains("uk") || v.contains("gb") {
        "en-GB".to_string()
    } else if v.contains("australian") || v.contains("au") {
        "en-AU".to_string()
    } else {
        "en".to_string()
    }
}

/// Synthesize `text` to MP3 bytes, chunking to respect the endpoint's limit.
pub async fn synthesize(text: &str, lang: &str) -> Result<Vec<u8>, String> {
    let chunks = split_text(text, MAX_CHUNK_CHARS);
    if chunks.is_empty() {
        return Err("keyless TTS: empty text".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| format!("keyless TTS client: {e}"))?;

    let speed = std::env::var("KEYLESS_TTS_SPEED").unwrap_or_else(|_| "1".to_string());
    let mut out: Vec<u8> = Vec::new();

    for (idx, chunk) in chunks.iter().enumerate() {
        let mut url = reqwest::Url::parse(ENDPOINT).map_err(|e| format!("keyless TTS url: {e}"))?;
        url.query_pairs_mut()
            .append_pair("ie", "UTF-8")
            .append_pair("q", chunk)
            .append_pair("tl", lang)
            .append_pair("client", "tw-ob")
            .append_pair("ttsspeed", &speed);

        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("keyless TTS request failed (chunk {}): {e}", idx + 1))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!(
                "keyless TTS HTTP {status} (chunk {}): {}",
                idx + 1,
                body.chars().take(120).collect::<String>()
            ));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("keyless TTS body failed (chunk {}): {e}", idx + 1))?;
        if bytes.is_empty() {
            return Err(format!("keyless TTS returned no audio (chunk {})", idx + 1));
        }

        // Each response starts with its own ID3v2 tag. Keep the first one so
        // players see valid metadata, strip it from the rest so the concatenated
        // stream stays a single continuous MP3. ID3v1 footers are stripped from
        // every chunk (they must only appear at the very end of a file).
        let body = strip_id3v1(&bytes);
        let body = if idx == 0 { body } else { strip_id3v2(body) };
        out.extend_from_slice(body);
    }

    if out.is_empty() {
        return Err("keyless TTS: empty audio".to_string());
    }
    Ok(out)
}

/// Split into <= `max` char chunks, preferring sentence then word boundaries so
/// the narration does not pause mid-phrase.
fn split_text(text: &str, max: usize) -> Vec<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current = String::new();

    for sentence in split_sentences(&normalized) {
        if sentence.chars().count() > max {
            // Long sentence: break on words.
            for word in sentence.split_whitespace() {
                if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > max {
                    chunks.push(std::mem::take(&mut current));
                }
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            }
            continue;
        }

        if !current.is_empty() && current.chars().count() + 1 + sentence.chars().count() > max {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(&sentence);
    }

    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Split on `.`/`!`/`?` while keeping the terminator with the sentence.
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        current.push(c);
        if matches!(c, '.' | '!' | '?') {
            // Include trailing quotes/brackets in the same sentence.
            while let Some(&next) = chars.peek() {
                if matches!(next, '"' | '\'' | ')' | ']' | '”' | '’') {
                    current.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                sentences.push(trimmed.to_string());
            }
            current.clear();
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_string());
    }
    sentences
}

fn strip_id3v1(data: &[u8]) -> &[u8] {
    if data.len() >= 128 && &data[data.len() - 128..data.len() - 125] == b"TAG" {
        &data[..data.len() - 128]
    } else {
        data
    }
}

fn strip_id3v2(data: &[u8]) -> &[u8] {
    if data.len() > 10 && &data[..3] == b"ID3" {
        // Synchsafe 28-bit size at bytes 6..10.
        let size = ((data[6] as usize & 0x7f) << 21)
            | ((data[7] as usize & 0x7f) << 14)
            | ((data[8] as usize & 0x7f) << 7)
            | (data[9] as usize & 0x7f);
        let mut total = 10 + size;
        // Footer present flag.
        if data[5] & 0x10 != 0 {
            total += 10;
        }
        if total < data.len() {
            return &data[total..];
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_respect_limit_and_word_boundaries() {
        let text = "One short sentence. Another sentence that is a bit longer, but still fine! And a third?";
        let chunks = split_text(text, 60);
        assert!(!chunks.is_empty());
        for c in &chunks {
            assert!(c.chars().count() <= 60, "chunk too long: {c}");
        }
    }

    #[test]
    fn very_long_sentence_breaks_on_words() {
        let word = "narration ";
        let text = word.repeat(60);
        let chunks = split_text(&text, 50);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.chars().count() <= 50);
            assert!(!c.starts_with(' '));
        }
    }
}
