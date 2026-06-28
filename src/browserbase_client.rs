use serde_json::Value;

const API_BASE: &str = "https://api.browserbase.com/v1";
const ENV_KEY: &str = "BROWSERBASE_API_KEY";
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_CONTENT_CHARS: usize = 8000;

fn api_key() -> Option<String> {
    std::env::var(ENV_KEY).ok()
}

fn headers() -> Option<reqwest::header::HeaderMap> {
    let key = api_key()?;
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(
        "X-BB-API-Key",
        reqwest::header::HeaderValue::from_str(&key).ok()?,
    );
    h.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    Some(h)
}

/// Fetch a URL via BrowserBase and return clean markdown content.
/// Returns `Ok(Some(content))` on success, `Ok(None)` if BrowserBase is not configured,
/// or `Err(error_message)` on failure.
pub async fn fetch_url(url: &str) -> Result<Option<String>, String> {
    if api_key().is_none() {
        return Ok(None);
    }

    let hdrs = headers().ok_or("Failed to build BrowserBase headers")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let resp = client
        .post(format!("{API_BASE}/fetch"))
        .headers(hdrs)
        .json(&serde_json::json!({
            "url": url,
            "format": "markdown",
        }))
        .send()
        .await
        .map_err(|e| format!("BrowserBase fetch request failed: {e}"))?;

    let status = resp.status();
    let data: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse BrowserBase response: {e}"))?;

    if !status.is_success() {
        let msg = data
            .get("message")
            .or_else(|| data.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("BrowserBase fetch failed (HTTP {status}): {msg}"));
    }

    let content = data
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if content.is_empty() {
        return Err(format!("BrowserBase returned empty content for: {url}"));
    }

    let truncated = if content.len() > MAX_CONTENT_CHARS {
        let mut s = String::with_capacity(MAX_CONTENT_CHARS + 50);
        s.push_str(&content[..MAX_CONTENT_CHARS]);
        s.push_str(&format!("\n\n[...truncated at {} chars]", MAX_CONTENT_CHARS));
        s
    } else {
        content
    };

    Ok(Some(truncated))
}

/// Check if BrowserBase is configured (API key is set).
pub fn is_configured() -> bool {
    api_key().is_some()
}
