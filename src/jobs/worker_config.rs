// Worker configuration module for parallel job processing
// Manages concurrency, polling intervals, and worker identity

use std::env;

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Number of concurrent jobs to process (default: 3)
    pub concurrency: usize,

    /// Polling interval in seconds (default: 30)
    pub poll_interval_secs: u64,

    /// Unique worker instance ID (format: hostname-pid-timestamp)
    pub worker_id: String,

    /// Minutes before a downloading job is considered stuck (default: 25)
    pub stuck_downloading_mins: u64,

    /// Minutes before an analyzing job is considered stuck (default: 10)
    pub stuck_analyzing_mins: u64,

    /// Minutes before a clip-extracting job is considered stuck (default: 15)
    pub stuck_extracting_mins: u64,

    /// Minutes before a posting job is considered stuck (default: 30)
    pub stuck_posting_mins: u64,

    /// Maximum retries before a job is discarded (default: 10)
    pub max_retries: u32,

    /// Initial backoff duration in seconds when quota is exhausted (default: 120)
    pub quota_pause_secs: u64,

    /// Minutes before an unclaimed pending job is considered stale enough to discard (default: 1440 = 24h)
    pub stale_pending_discard_mins: u64,
}

impl WorkerConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let concurrency = env::var("CLIPPING_WORKER_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        let poll_interval_secs = env::var("CLIPPING_WORKER_POLL_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let stuck_downloading_mins = env::var("CLIPPING_STUCK_DOWNLOADING_MINS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(25);

        let stuck_analyzing_mins = env::var("CLIPPING_STUCK_ANALYZING_MINS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let stuck_extracting_mins = env::var("CLIPPING_STUCK_EXTRACTING_MINS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(15);

        let stuck_posting_mins = env::var("CLIPPING_STUCK_POSTING_MINS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let max_retries = env::var("CLIPPING_MAX_RETRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let quota_pause_secs = env::var("CLIPPING_QUOTA_PAUSE_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120);

        let stale_pending_discard_mins = env::var("CLIPPING_STALE_PENDING_DISCARD_MINS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1440);

        // Generate unique worker ID: hostname-pid-timestamp
        let worker_id = format!(
            "{}-{}-{}",
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "unknown".to_string()),
            std::process::id(),
            chrono::Utc::now().timestamp()
        );

        Self {
            concurrency,
            poll_interval_secs,
            worker_id,
            stuck_downloading_mins,
            stuck_analyzing_mins,
            stuck_extracting_mins,
            stuck_posting_mins,
            max_retries,
            quota_pause_secs,
            stale_pending_discard_mins,
        }
    }

    /// Validate configuration parameters
    pub fn validate(&self) -> Result<(), String> {
        if self.concurrency == 0 {
            return Err("CLIPPING_WORKER_CONCURRENCY must be at least 1".to_string());
        }

        if self.concurrency > 10 {
            return Err(
                "CLIPPING_WORKER_CONCURRENCY must not exceed 10 (resource protection)".to_string(),
            );
        }

        if self.poll_interval_secs == 0 {
            return Err("CLIPPING_WORKER_POLL_INTERVAL must be at least 1 second".to_string());
        }

        if self.poll_interval_secs > 300 {
            tracing::warn!(
                "CLIPPING_WORKER_POLL_INTERVAL is {}s (>5 minutes). Jobs may be delayed.",
                self.poll_interval_secs
            );
        }

        if self.stale_pending_discard_mins < 60 {
            return Err(
                "CLIPPING_STALE_PENDING_DISCARD_MINS must be at least 60 minutes".to_string(),
            );
        }

        Ok(())
    }

    /// Get recommended database connection pool size
    /// Formula: 5 (API) + concurrency + 2 (buffer)
    pub fn recommended_db_pool_size(&self) -> u32 {
        5 + self.concurrency as u32 + 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        // Remove env vars if set
        env::remove_var("CLIPPING_WORKER_CONCURRENCY");
        env::remove_var("CLIPPING_WORKER_POLL_INTERVAL");

        let config = WorkerConfig::from_env();
        assert_eq!(config.concurrency, 3);
        assert_eq!(config.poll_interval_secs, 30);
        assert_eq!(config.stuck_downloading_mins, 25);
        assert_eq!(config.stuck_analyzing_mins, 10);
        assert_eq!(config.stuck_extracting_mins, 15);
        assert_eq!(config.stuck_posting_mins, 30);
        assert_eq!(config.max_retries, 10);
        assert_eq!(config.quota_pause_secs, 120);
        assert!(!config.worker_id.is_empty());
    }

    #[test]
    fn test_validation() {
        let mut config = WorkerConfig {
            concurrency: 3,
            poll_interval_secs: 30,
            worker_id: "test-worker".to_string(),
            stuck_downloading_mins: 25,
            stuck_analyzing_mins: 10,
            stuck_extracting_mins: 15,
            stuck_posting_mins: 30,
            max_retries: 10,
            quota_pause_secs: 120,
            stale_pending_discard_mins: 1440,
        };

        assert!(config.validate().is_ok());

        // Test invalid concurrency (zero)
        config.concurrency = 0;
        assert!(config.validate().is_err());

        // Test invalid concurrency (too high)
        config.concurrency = 11;
        assert!(config.validate().is_err());

        // Valid concurrency
        config.concurrency = 5;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_recommended_pool_size() {
        let config = WorkerConfig {
            concurrency: 3,
            poll_interval_secs: 30,
            worker_id: "test".to_string(),
            stuck_downloading_mins: 25,
            stuck_analyzing_mins: 10,
            stuck_extracting_mins: 15,
            stuck_posting_mins: 30,
            max_retries: 10,
            quota_pause_secs: 120,
            stale_pending_discard_mins: 1440,
        };

        // 5 (API) + 3 (workers) + 2 (buffer) = 10
        assert_eq!(config.recommended_db_pool_size(), 10);
    }
}
