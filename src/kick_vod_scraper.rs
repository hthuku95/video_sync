use regex::Regex;

/// Resolve any Kick URL (channel or VOD) to an HLS stream URL.
///
/// Takes a URL like:
/// - `https://kick.com/{slug}` (channel, list VODs → get latest)
/// - `https://kick.com/{slug}/videos/{uuid}` (specific VOD, extract HLS)
/// - `https://kick.com/{slug}/video/{uuid}` (singular, same as above)
///
/// For non-Kick URLs, returns the input unchanged.
pub async fn resolve_url_to_hls(url: &str) -> String {
    if !url.contains("kick.com") {
        return url.to_string();
    }

    // Check if it's already a VOD URL (contains /videos/ or /video/ + UUID)
    let has_uuid = Regex::new(r"/videos?/[a-f0-9-]{8,}")
        .ok()
        .map(|re| re.is_match(url))
        .unwrap_or(false);

    if has_uuid {
        // Direct VOD URL — extract HLS stream
        match extract_stream_url(url).await {
            Ok(hls) => {
                tracing::info!("Kick VOD → HLS: {}", hls);
                return hls;
            }
            Err(e) => {
                tracing::warn!("Kick HLS extraction failed for VOD URL: {} — {}. Using original URL.", url, e);
                return url.to_string();
            }
        }
    }

    // Channel URL — resolve latest VOD, then extract HLS
    let slug = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");

    match resolve_latest_stream(slug).await {
        Ok((_vod_url, hls_url)) => {
            tracing::info!("Kick channel {} → latest VOD → HLS: {}", slug, hls_url);
            hls_url
        }
        Err(e) => {
            tracing::warn!("Kick VOD resolution failed for channel '{}': {}. Using original URL.", slug, e);
            url.to_string()
        }
    }
}

/// Fetch the list of VOD URLs for a Kick channel using BrowserBase.
/// Returns the most recent VOD URL (first on the videos page).
pub async fn list_vods(slug: &str) -> Result<Vec<String>, String> {
    let url = format!("https://kick.com/{slug}/videos/");
    match crate::browserbase_client::fetch_url_raw(&url).await {
        Ok(Some(html)) => {
            let re = Regex::new(r#"/\w[\w-]*/videos?/([a-f0-9-]+)"#)
                .map_err(|e| format!("Regex error: {e}"))?;

            let mut vods: Vec<String> = Vec::new();
            for cap in re.captures_iter(&html) {
                if let Some(id) = cap.get(1) {
                    let vod_url = format!("https://kick.com/{slug}/videos/{}", id.as_str());
                    if !vods.contains(&vod_url) {
                        vods.push(vod_url);
                    }
                }
            }
            if !vods.is_empty() {
                return Ok(vods);
            }
            tracing::warn!("Kick VOD listing: BrowserBase returned page with no VODs for '{}'", slug);
        }
        Ok(None) => {
            tracing::warn!("Kick VOD listing: BrowserBase not configured for '{}'", slug);
        }
        Err(e) => {
            // 402 (credits exhausted) etc. — logged; fall through to yt-dlp.
            tracing::warn!("Kick VOD listing via BrowserBase failed for '{}': {} — trying yt-dlp fallback", slug, e);
        }
    }

    // ── BROWSERBASE-FREE FALLBACK (Sep 3 2026) ──
    // BrowserBase 402 (credits exhausted) silently broke ALL Kick VOD
    // resolution — the root cause of the campaign renders improvising
    // non-Kick sources. yt-dlp --flat-playlist resolves Kick channel VOD
    // lists directly (verified live from the ytdlp node: returns the latest
    // VOD's HLS playlist without any browser infra).
    let out = tokio::process::Command::new("yt-dlp")
        .args([
            "--flat-playlist",
            "--print", "url",
            "--playlist-items", "5",
            &url,
        ])
        .output()
        .await
        .map_err(|e| format!("yt-dlp spawn failed: {e}"))?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    // yt-dlp --flat-playlist on a Kick channel returns HLS (.m3u8) playlist
    // URLs directly (verified live) — one per VOD, newest first. These ARE
    // the resolved streams; the caller's extract_stream_url step is skipped
    // for them because resolve_latest_stream passes each entry through
    // extract_stream_url, which will find nothing more to do for an m3u8 —
    // so filter the other way: keep everything, but mark m3u8 entries so the
    // caller can skip re-extraction.
    let vods: Vec<String> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("http"))
        .map(|l| {
            if l.contains(".m3u8") {
                format!("{}#direct-hls", l)
            } else {
                l.to_string()
            }
        })
        .collect();

    if !out.status.success() || vods.is_empty() {
        return Err(format!(
            "No VODs found for Kick channel '{}' — BrowserBase failed AND yt-dlp fallback found none (status {})",
            slug, out.status
        ));
    }

    tracing::info!("Kick VOD listing via yt-dlp fallback: {} VODs for '{}'", vods.len(), slug);
    Ok(vods)
}

/// Extract the HLS/m3u8 stream URL from a Kick VOD page using BrowserBase.
///
/// Kick VOD pages embed the stream source in a `<script>` tag as JSON
/// (server-rendered), so BrowserBase's fetch with `format: raw` can capture it
/// without needing a full browser session.
pub async fn extract_stream_url(vod_url: &str) -> Result<String, String> {
    // yt-dlp fallback entries already carry their resolved HLS URL
    // (marked by list_vods) — no BrowserBase extraction needed.
    if let Some(rest) = vod_url.strip_suffix("#direct-hls") {
        return Ok(rest.to_string());
    }

    let html = crate::browserbase_client::fetch_url_raw(vod_url)
        .await?
        .ok_or_else(|| "BrowserBase not configured".to_string())?;

    // Pattern 1: "source":"https://...m3u8" or "src":"https://...m3u8"
    let patterns = [
        // "source":"https://...m3u8"
        r#""source"\s*:\s*"([^"]+\.m3u8[^"]*)""#,
        // "src":"https://...m3u8"
        r#""src"\s*:\s*"([^"]+\.m3u8[^"]*)""#,
        // "playbackUrl":"https://...m3u8"
        r#""playbackUrl"\s*:\s*"([^"]+\.m3u8[^"]*)""#,
        // "url":"https://...m3u8"
        r#""url"\s*:\s*"([^"]+\.m3u8[^"]*)""#,
        // Direct <video src="https://...m3u8">
        r#"<video[^>]*src="([^"]+\.m3u8[^"]*)""#,
        // Direct <source src="https://...m3u8">
        r#"<source[^>]*src="([^"]+\.m3u8[^"]*)""#,
    ];

    for pattern in &patterns {
        let re = Regex::new(pattern).map_err(|e| format!("Regex error: {e}"))?;
        if let Some(cap) = re.captures(&html) {
            if let Some(m) = cap.get(1) {
                let stream_url = m.as_str().to_string();
                // Unescape any JSON unicode escapes
                let stream_url = stream_url.replace("\\/", "/");
                if stream_url.starts_with("http") && stream_url.contains(".m3u8") {
                    return Ok(stream_url);
                }
            }
        }
    }

    // Pattern N: try to find any m3u8 URL in the page
    let fallback_re = Regex::new(r#"https?://[^"'\s]+\.m3u8[^"'\s]*"#)
        .map_err(|e| format!("Regex error: {e}"))?;
    if let Some(m) = fallback_re.find(&html) {
        return Ok(m.as_str().to_string());
    }

    Err(format!(
        "Could not find HLS stream URL in Kick VOD page: {vod_url}"
    ))
}

/// Resolve a Kick channel URL to the latest working VOD HLS stream URL.
/// Tries the 5 most recent VODs in order. If all fail, returns an error
/// so the caller can fall back to yt-dlp or the raw channel URL.
/// Returns `(vod_url, hls_url)`.
pub async fn resolve_latest_stream(slug: &str) -> Result<(String, String), String> {
    let vods = list_vods(slug).await?;

    // Try up to 5 most recent VODs
    let max_attempts = 5usize.min(vods.len());
    let mut last_error = String::new();
    for vod in vods.into_iter().take(max_attempts) {
        match extract_stream_url(&vod).await {
            Ok(hls) => {
                tracing::info!("Kick VOD → HLS (attempt #{}/{}): {}", max_attempts - max_attempts + 1, max_attempts, hls);
                return Ok((vod, hls));
            }
            Err(e) => {
                tracing::warn!("Kick VOD failed (attempt #{}/{}): {} — {}. Trying next...",
                    max_attempts - max_attempts + 1, max_attempts, vod, e);
                last_error = e;
            }
        }
    }

    // Log channel URL as fallback for yt-dlp
    let channel_url = format!("https://kick.com/{}", slug);
    tracing::warn!(
        "All {} VOD attempts failed for '{}'. Last error: {}. yt-dlp may resolve the channel URL directly: {}",
        max_attempts, slug, last_error, channel_url
    );
    Err(format!(
        "No working VOD HLS found for '{}' (tried {} VODs). Last error: {}. \
         Fallback: return channel URL for yt-dlp: {}",
        slug, max_attempts, last_error, channel_url
    ))
}
