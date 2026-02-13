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
        }
    }

    /// Validate configuration parameters
    pub fn validate(&self) -> Result<(), String> {
        if self.concurrency == 0 {
            return Err("CLIPPING_WORKER_CONCURRENCY must be at least 1".to_string());
        }

        if self.concurrency > 10 {
            return Err(
                "CLIPPING_WORKER_CONCURRENCY must not exceed 10 (resource protection)".to_string()
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
        assert!(!config.worker_id.is_empty());
    }

    #[test]
    fn test_validation() {
        let mut config = WorkerConfig {
            concurrency: 3,
            poll_interval_secs: 30,
            worker_id: "test-worker".to_string(),
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
        };

        // 5 (API) + 3 (workers) + 2 (buffer) = 10
        assert_eq!(config.recommended_db_pool_size(), 10);
    }
}
