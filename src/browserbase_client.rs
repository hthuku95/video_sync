use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

const API_BASE: &str = "https://api.browserbase.com/v1";
const ENV_KEY: &str = "BROWSERBASE_API_KEY";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_CONTENT_CHARS: usize = 8000;
const MAX_CRAWL_PAGES: usize = 15;

/// A single page fetched during crawl.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CrawledPage {
    pub url: String,
    pub title: String,
    pub content: String,
}

/// Result of crawling a website via BrowserBase.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CrawlResult {
    pub pages: Vec<CrawledPage>,
    pub combined_markdown: String,
    pub css_info: String,
    pub feature_tag: String,
}

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

/// Fetch a URL via BrowserBase and return the full raw HTML content (no truncation).
/// Useful for parsing structured data embedded in <script> tags on server-rendered pages.
pub async fn fetch_url_raw(url: &str) -> Result<Option<String>, String> {
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
            "format": "raw",
        }))
        .send()
        .await
        .map_err(|e| format!("BrowserBase raw fetch request failed: {e}"))?;

    let status = resp.status();
    let data: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse BrowserBase raw response: {e}"))?;

    if !status.is_success() {
        let msg = data
            .get("message")
            .or_else(|| data.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        let bb_err = format!("BrowserBase raw fetch failed (HTTP {status}): {msg}");
        // ── SCRAPLING FALLBACK (Sep 4 2026) ──
        // BrowserBase 402 (credits exhausted) silently broke Kick VOD
        // resolution and cascaded into publishing a wrong video. Fall back
        // to our self-hosted Scrapling service (ytdlp node) on ANY failure.
        tracing::warn!("{bb_err} — trying Scrapling fallback");
        return scrapling_fetch_raw(url)
            .await
            .map_err(|e| format!("{bb_err}; Scrapling fallback also failed: {e}"));
    }

    let content = data
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if content.is_empty() {
        let bb_err = format!("BrowserBase returned empty raw content for: {url}");
        tracing::warn!("{bb_err} — trying Scrapling fallback");
        return scrapling_fetch_raw(url)
            .await
            .map_err(|e| format!("{bb_err}; Scrapling fallback also failed: {e}"));
    }

    Ok(Some(content))
}

/// Self-hosted Scrapling fetch fallback (ytdlp node, Option B — served
/// through the same YTDLP_API_URL service as the downloader). Returns raw
/// HTML with the same contract as fetch_url_raw.
async fn scrapling_fetch_raw(url: &str) -> Result<Option<String>, String> {
    let base = std::env::var("YTDLP_API_URL")
        .map_err(|_| "YTDLP_API_URL not set — Scrapling fallback unavailable".to_string())?;
    let base = base.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let resp = client
        .post(format!("{base}/api/v1/fetch"))
        .json(&serde_json::json!({ "url": url }))
        .send()
        .await
        .map_err(|e| format!("Scrapling request failed: {e}"))?;

    let status = resp.status();
    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Scrapling response parse failed: {e}"))?;

    if !status.is_success() || data.get("success") != Some(&serde_json::Value::Bool(true)) {
        let detail = data
            .get("detail")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("Scrapling fetch failed (HTTP {status}): {detail}"));
    }

    let html = data
        .get("html")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if html.is_empty() {
        return Err(format!("Scrapling returned empty html for: {url}"));
    }

    tracing::info!(
        "✅ Scrapling fallback fetched {} ({} bytes, mode={})",
        url,
        html.len(),
        data.get("mode").and_then(|m| m.as_str()).unwrap_or("?")
    );
    Ok(Some(html))
}

/// Check if BrowserBase is configured (API key is set).
pub fn is_configured() -> bool {
    api_key().is_some()
}

/// Normalize a URL for deduplication — strip trailing slash, lowercase host.
fn normalize_url(url: &str) -> String {
    let url = url.trim_end_matches('/');
    url.to_lowercase()
}

/// Extract CSS design tokens from raw HTML.
/// Returns structured text describing colors, fonts, and other design patterns.
fn extract_css_info(html: &str) -> String {
    let mut colors = Vec::new();
    let mut fonts = Vec::new();
    let mut seen_colors = std::collections::HashSet::new();
    let mut seen_fonts = std::collections::HashSet::new();

    // Extract <style> block contents
    let style_re = Regex::new(r#"<style[^>]*>(.*?)</style>"#).unwrap();
    for cap in style_re.captures_iter(html) {
        let css_text = &cap[1];
        extract_css_tokens(css_text, &mut colors, &mut seen_colors, &mut fonts, &mut seen_fonts);
    }

    // Extract inline style="..." attributes
    let inline_re = Regex::new(r#"style\s*=\s*"([^"]*?)""#).unwrap();
    for cap in inline_re.captures_iter(html) {
        extract_css_tokens(&cap[1], &mut colors, &mut seen_colors, &mut fonts, &mut seen_fonts);
    }

    let color_summary = if colors.is_empty() {
        String::new()
    } else {
        format!("Colors: {}", colors.join(", "))
    };

    let font_summary = if fonts.is_empty() {
        String::new()
    } else {
        format!("Fonts: {}", fonts.join(", "))
    };

    let mut parts = Vec::new();
    if !color_summary.is_empty() {
        parts.push(color_summary);
    }
    if !font_summary.is_empty() {
        parts.push(font_summary);
    }
    parts.join("\n")
}

fn extract_css_tokens(
    css: &str,
    colors: &mut Vec<String>,
    seen_colors: &mut std::collections::HashSet<String>,
    fonts: &mut Vec<String>,
    seen_fonts: &mut std::collections::HashSet<String>,
) {
    // Hex colors: #rgb, #rrggbb, #rrggbbaa
    let hex_re = Regex::new(r"#([0-9a-fA-F]{3,8})\b").unwrap();
    for cap in hex_re.captures_iter(css) {
        let c = format!("#{}", &cap[1]);
        if seen_colors.insert(c.clone()) {
            colors.push(c);
        }
    }

    // rgb/rgba colors
    let rgb_re = Regex::new(r"rgba?\s*\(\s*\d+\s*,\s*\d+\s*,\s*\d+[^)]*\)").unwrap();
    for m in rgb_re.find_iter(css) {
        let c = m.as_str().to_string();
        if seen_colors.insert(c.clone()) {
            colors.push(c);
        }
    }

    // font-family declarations
    let font_re = Regex::new(r#"(?i)font-family\s*:\s*([^;}]+)"#).unwrap();
    for cap in font_re.captures_iter(css) {
        for family in cap[1].split(',') {
            let f = family.trim().trim_matches('\'').trim_matches('"').to_string();
            if !f.is_empty() && f != "sans-serif" && f != "serif" && f != "monospace" && seen_fonts.insert(f.clone()) {
                fonts.push(f);
            }
        }
    }
}

/// Attempt to fetch a linked stylesheet and extract tokens from it.
async fn fetch_and_extract_stylesheet(url: &str, colors: &mut Vec<String>, fonts: &mut Vec<String>) {
    let mut seen_colors = std::collections::HashSet::new();
    let mut seen_fonts = std::collections::HashSet::new();
    if let Ok(Some(css_content)) = fetch_url_raw(url).await {
        if !css_content.starts_with("<") { // skip if it returned HTML instead of CSS
            extract_css_tokens(&css_content, colors, &mut seen_colors, fonts, &mut seen_fonts);
        }
    }
}

/// Crawl a website: fetch the homepage, extract internal links, fetch each subpage.
/// Returns structured markdown + CSS info + a feature tag for Qdrant vectorization.
pub async fn crawl_website(url: &str) -> Result<CrawlResult, String> {
    if api_key().is_none() {
        return Err("BrowserBase not configured (BROWSERBASE_API_KEY not set)".to_string());
    }

    let full_url = if !url.starts_with("http") {
        format!("https://{url}")
    } else {
        url.to_string()
    };

    let base_url = Url::parse(&full_url).map_err(|e| format!("Invalid URL '{full_url}': {e}"))?;
    let base_host = base_url.host_str().ok_or_else(|| format!("No host in URL '{full_url}'"))?.to_string();

    // Feature tag: deterministic from base URL
    let hash = hex::encode(Sha256::digest(full_url.as_bytes()));
    let feature_tag = format!("crawled_site_{}", &hash[..12]);

    // Step 1: Fetch homepage raw HTML for link extraction
    let homepage_html = fetch_url_raw(&full_url)
        .await?
        .ok_or_else(|| "BrowserBase returned empty homepage".to_string())?;

    // Step 2: Extract internal links
    let href_re = Regex::new(r#"<a\s[^>]*href="([^"]*)""#).unwrap();
    let mut internal_urls = std::collections::BTreeSet::new(); // ordered + deduped

    for cap in href_re.captures_iter(&homepage_html) {
        let href = &cap[1];
        // Skip empty, fragments, javascript, mailto, tel, file downloads
        if href.is_empty()
            || href.starts_with('#')
            || href.starts_with("javascript:")
            || href.starts_with("mailto:")
            || href.starts_with("tel:")
        {
            continue;
        }

        // Resolve relative URLs against base
        let absolute = if href.starts_with("http") {
            match Url::parse(href) {
                Ok(u) => u,
                Err(_) => continue,
            }
        } else {
            match base_url.join(href) {
                Ok(u) => u,
                Err(_) => continue,
            }
        };

        // Same host only
        let abs_host = match absolute.host_str() {
            Some(h) => h.to_string(),
            None => continue,
        };
        if abs_host != base_host {
            continue;
        }

        // Skip file downloads
        let path = absolute.path();
        let skip_extensions = [".pdf", ".zip", ".mp4", ".mp3", ".png", ".jpg", ".jpeg",
                               ".gif", ".svg", ".webp", ".ico", ".woff", ".woff2", ".ttf",
                               ".eot", ".mov", ".avi", ".mkv", ".doc", ".docx", ".xls", ".xlsx"];
        if skip_extensions.iter().any(|ext| path.ends_with(ext)) {
            continue;
        }

        // Remove fragment
        let mut clean = absolute.clone();
        clean.set_fragment(None);

        // Remove trailing slash for consistency
        let url_str = clean.as_str().trim_end_matches('/').to_string();
        internal_urls.insert(url_str);
    }

    // Limit to MAX_CRAWL_PAGES (excluding homepage which we already have)
    let subpages: Vec<String> = internal_urls.into_iter().take(MAX_CRAWL_PAGES).collect();

    // Step 3: Extract CSS info from homepage HTML
    let mut colors = Vec::new();
    let mut fonts = Vec::new();
    let mut seen_c = std::collections::HashSet::new();
    let mut seen_f = std::collections::HashSet::new();
    extract_css_tokens(&homepage_html, &mut colors, &mut seen_c, &mut fonts, &mut seen_f);

    // Fetch linked stylesheets
    let link_re = Regex::new(r#"<link[^>]*href="([^"]*\.css[^"]*)"[^>]*>"#).unwrap();
    for cap in link_re.captures_iter(&homepage_html) {
        let css_url_str = &cap[1];
        let css_absolute = if css_url_str.starts_with("http") {
            css_url_str.to_string()
        } else if css_url_str.starts_with("//") {
            format!("https:{}", css_url_str)
        } else {
            match base_url.join(css_url_str) {
                Ok(u) => u.as_str().to_string(),
                Err(_) => continue,
            }
        };
        fetch_and_extract_stylesheet(&css_absolute, &mut colors, &mut fonts).await;
    }

    let css_info = {
        let mut parts = Vec::new();
        if !colors.is_empty() {
            parts.push(format!("Colors: {}", colors.join(", ")));
        }
        if !fonts.is_empty() {
            parts.push(format!("Fonts: {}", fonts.join(", ")));
        }
        parts.join("\n")
    };

    // Step 4: Fetch homepage markdown
    let homepage_md = fetch_url(&full_url)
        .await?
        .unwrap_or_default();

    // Extract title from homepage HTML
    let title_re = Regex::new(r#"(?i)<title>([^<]+)"#).unwrap();
    let homepage_title = title_re.captures(&homepage_html)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| "Home".to_string());

    let mut pages = Vec::new();
    pages.push(CrawledPage {
        url: full_url.clone(),
        title: homepage_title.clone(),
        content: homepage_md.clone(),
    });

    // Step 5: Fetch each subpage in parallel
    let title_re_for_closure = title_re.clone();
    let subpage_fetches: Vec<_> = subpages.iter().map(|sub_url| {
        let tr = title_re_for_closure.clone();
        async move {
            let md = fetch_url(sub_url).await.ok().flatten().unwrap_or_default();
            let html = fetch_url_raw(sub_url).await.ok().flatten().unwrap_or_default();
            let title = tr.captures(&html)
                .map(|c| c[1].to_string())
                .unwrap_or_else(|| sub_url.rsplit('/').next().unwrap_or("page").to_string());
            CrawledPage {
                url: sub_url.clone(),
                title,
                content: md,
            }
        }
    }).collect::<Vec<_>>();

    // Collect results
    let mut page_title_lines = Vec::new();
    page_title_lines.push(format!("# {}\n\n{}", homepage_title, homepage_md));

    let mut subpage_results = Vec::new();
    for fetch in subpage_fetches {
        let page = fetch.await;
        if !page.content.is_empty() {
            page_title_lines.push(format!("\n\n---\n\n# {} ({})", page.title, page.url));
            page_title_lines.push(page.content.clone());
            subpage_results.push(page);
        }
    }

    pages.extend(subpage_results);

    let combined_markdown = page_title_lines.join("");

    Ok(CrawlResult {
        pages,
        combined_markdown,
        css_info,
        feature_tag,
    })
}
