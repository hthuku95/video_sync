/// Error classification for clipping job failures.
///
/// Determines whether a job failure is permanent (never retry), quota-related
/// (retry after extended backoff), or transient (retry with exponential backoff).
#[derive(Debug, PartialEq)]
pub enum ErrorClass {
    /// Permanent: no point retrying. Set status = 'cancelled'.
    /// Examples: private video, invalid OAuth grant, UUID parse errors.
    Permanent,
    /// Quota: rate-limited by external API. Retry after extended backoff.
    /// Examples: Gemini RESOURCE_EXHAUSTED, YouTube 429.
    Quota,
    /// Transient: network/service blip. Retry with exponential backoff.
    /// Examples: timeout, 502/503, connection reset.
    Transient,
}

pub fn classify(err: &str) -> ErrorClass {
    // Permanent — content or credential issues, retrying is pointless
    let permanent_patterns = [
        "Video is private",
        "video has been removed",
        "This video is unavailable",
        "Video unavailable",
        "invalid_grant",
        "Token has been expired or revoked",
        "403 Forbidden",
        "Unable to parse UUID",
        "No Twitch mapping exists",
        "Cannot fetch content from the provided URL",
        "Request contains an invalid argument",
    ];
    for p in permanent_patterns {
        if err.contains(p) {
            return ErrorClass::Permanent;
        }
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
