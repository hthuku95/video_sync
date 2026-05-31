// Background job for periodic YouTube Analytics synchronization
// Implements the data collection part of the Feedback Loop (Recommendation 5)

use crate::clipping::performance_tracker::PerformanceTracker;
use sqlx::PgPool;
use tokio::time::{interval, Duration};

pub struct AnalyticsSyncJob {
    db_pool: PgPool,
    performance_tracker: PerformanceTracker,
}

impl AnalyticsSyncJob {
    pub fn new(db_pool: PgPool) -> Self {
        let performance_tracker = PerformanceTracker::new(db_pool.clone());
        Self {
            db_pool,
            performance_tracker,
        }
    }

    /// Start the analytics sync job (runs every 6 hours)
    /// This should be spawned as a background task in main.rs
    pub async fn start(self) {
        tracing::info!("🚀 Starting Analytics Sync Job (runs every 6 hours)");

        let mut interval_timer = interval(Duration::from_secs(6 * 3600)); // 6 hours

        loop {
            interval_timer.tick().await;

            tracing::info!("⏰ Analytics sync triggered");

            match self.performance_tracker.sync_all_clip_analytics().await {
                Ok(report) => {
                    tracing::info!(
                        "✅ Analytics sync completed: {} clips synced, {} failed, took {}s",
                        report.clips_synced,
                        report.clips_failed,
                        report.duration_seconds
                    );

                    // Log to database for monitoring
                    if let Err(e) = self.log_sync_report(&report).await {
                        tracing::error!("Failed to log sync report: {}", e);
                    }
                }
                Err(e) => {
                    tracing::error!("❌ Analytics sync failed: {}", e);
                }
            }
        }
    }

    /// Log sync report to database for monitoring
    async fn log_sync_report(
        &self,
        report: &crate::clipping::performance_tracker::AnalyticsSyncReport,
    ) -> Result<(), String> {
        let query = "
            INSERT INTO analytics_sync_log (
                clips_synced,
                clips_failed,
                duration_seconds,
                sync_completed_at
            ) VALUES ($1, $2, $3, NOW())
        ";

        sqlx::query(query)
            .bind(report.clips_synced as i32)
            .bind(report.clips_failed as i32)
            .bind(report.duration_seconds as i32)
            .execute(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to log sync report: {}", e))?;

        Ok(())
    }
}

// Note: Add this table to a future migration if monitoring is needed:
// CREATE TABLE analytics_sync_log (
//     id SERIAL PRIMARY KEY,
//     clips_synced INTEGER NOT NULL,
//     clips_failed INTEGER NOT NULL,
//     duration_seconds INTEGER NOT NULL,
//     sync_completed_at TIMESTAMPTZ NOT NULL,
//     created_at TIMESTAMPTZ DEFAULT NOW()
// );
