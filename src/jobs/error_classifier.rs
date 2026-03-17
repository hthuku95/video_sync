/// Error classification for clipping job failures.
///
/// Determines whether a job failure is permanent (never retry), quota-related
/// (retry after extended backoff), OAuth-related (retry only after reconnection),
/// or transient (retry with exponential backoff).
#[derive(Debug, PartialEq)]
pub enum ErrorClass {
    /// Permanent: no point retrying. Set status = 'cancelled'.
    /// Examples: private video, UUID parse errors, Gemini invalid-argument.
    Permanent,
    /// Quota: rate-limited by external API. Retry after extended backoff.
    /// Examples: Gemini RESOURCE_EXHAUSTED, YouTube 429.
    Quota,
    /// OAuthExpired: destination channel token expired or revoked.
    /// Retry is pointless until the user reconnects their channel (requires_reauth → false).
    /// The auto_retry gate in clipping_worker skips these jobs while requires_reauth=true,
    /// and picks them up automatically once the channel is reconnected.
    OAuthExpired,
    /// Transient: network/service blip. Retry with exponential backoff.
    /// Examples: timeout, 502/503, connection reset.
    Transient,
}

pub fn classify(err: &str) -> ErrorClass {
    // Permanent — content or configuration issues, retrying is pointless
    let permanent_patterns = [
        "Video is private",
        "video has been removed",
        "This video is unavailable",
        "Video unavailable",
        "Unable to parse UUID",
        "No Twitch mapping exists",
        "Cannot fetch content from the provided URL",
        "Request contains an invalid argument",
        // Twitch VOD deleted/expired — rusty_ytdl returns this for missing content
        "The video not found",
        "video not found",
    ];
    for p in permanent_patterns {
        if err.contains(p) {
            return ErrorClass::Permanent;
        }
    }

    // OAuth expired — channel needs reconnection before this job can succeed.
    // Do NOT cancel: job should be retried once the user reconnects.
    if err.contains("authorization expired")
        || err.contains("Token refresh failed")
        || err.contains("needs reconnection")
        || err.contains("invalid_grant")
        || err.contains("Token has been expired or revoked")
        || err.contains("403 Forbidden")
    {
        return ErrorClass::OAuthExpired;
    }

    // Quota — back off long, don't spam the API
    if err.contains("RESOURCE_EXHAUSTED")
        || err.contains("quota exceeded")
        || err.contains("Quota exceeded")
        || err.contains("Resource has been exhausted")
        || err.contains("429")
        || err.contains("Too Many Requests")
    {
        return ErrorClass::Quota;
    }

    ErrorClass::Transient
}
