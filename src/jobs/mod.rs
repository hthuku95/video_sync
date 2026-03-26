// src/jobs/mod.rs
//! Background job system for video editing tasks
//! Enables non-blocking video processing with real-time progress updates via WebSocket

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};

pub mod video_job;
pub mod clipping_job;
pub mod clipping_worker;
pub mod analytics_sync_job;
pub mod token_refresh;
pub mod worker_config;
pub mod job_claimer;
pub mod twitch_mapper_job;
pub mod error_classifier;

pub use analytics_sync_job::AnalyticsSyncJob;

/// Unique identifier for a background job
pub type JobId = String;

/// Job status representing the current state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum JobStatus {
    /// Job is queued and waiting to start
    Queued {
        position: usize,
    },
    /// Job is currently running
    Running {
        current_step: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        progress_percent: Option<f64>,  // Now optional - use only when meaningful
        steps_completed: usize,
        total_steps: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        completed_actions: Option<Vec<String>>,  // NEW: List of completed steps for visibility
        #[serde(skip_serializing_if = "Option::is_none")]
        current_action_detail: Option<String>,  // NEW: Detailed sub-step info
    },
    /// Job completed successfully
    Completed {
        result: String,
        output_files: Vec<String>,
        duration_seconds: f64,
    },
    /// Job failed with error
    Failed {
        error: String,
        failed_at_step: String,
    },
    /// Job was paused by user
    Paused {
        paused_at_step: String,
        progress_percent: f64,
    },
    /// Job was cancelled by user
    Cancelled {
        cancelled_at_step: String,
    },
}

/// Progress update message sent to WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressUpdate {
    pub job_id: JobId,
    pub timestamp: DateTime<Utc>,
    pub message: String,
    pub status: JobStatus,
    pub details: Option<serde_json::Value>,
}

impl ProgressUpdate {
    pub fn new(job_id: JobId, message: String, status: JobStatus) -> Self {
        Self {
            job_id,
            timestamp: Utc::now(),
            message,
            status,
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Job metadata and control structure
#[derive(Debug, Clone)]
pub struct Job {
    pub id: JobId,
    pub session_id: String,
    pub user_id: Option<String>,
    pub job_type: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub last_heartbeat: DateTime<Utc>,  // NEW: Track last progress update to detect stuck jobs
    pub status: JobStatus,
    pub input_data: serde_json::Value,
}

impl Job {
    pub fn new(session_id: String, job_type: String, input_data: serde_json::Value) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            session_id,
            user_id: None,
            job_type,
            created_at: now,
            started_at: None,
            completed_at: None,
            last_heartbeat: now,
            status: JobStatus::Queued { position: 0 },
            input_data,
        }
    }

    pub fn with_user_id(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Check if job appears to be stuck (no heartbeat for > 10 minutes while running)
    pub fn is_possibly_stuck(&self) -> bool {
        match &self.status {
            JobStatus::Running { .. } => {
                let elapsed = Utc::now() - self.last_heartbeat;
                elapsed.num_minutes() > 10
            }
            _ => false,
        }
    }
}

/// Control commands for managing jobs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobControl {
    Pause,
    Resume,
    Cancel,
    UpdateInput(serde_json::Value),
}

/// Job manager handles background job execution and state
pub struct JobManager {
    /// Active jobs indexed by job_id
    jobs: Arc<RwLock<HashMap<JobId, Job>>>,
    /// Progress senders indexed by session_id (for WebSocket delivery)
    progress_senders: Arc<RwLock<HashMap<String, mpsc::UnboundedSender<ProgressUpdate>>>>,
    /// Control channels for each job
    control_channels: Arc<RwLock<HashMap<JobId, mpsc::UnboundedSender<JobControl>>>>,
}

impl JobManager {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            progress_senders: Arc::new(RwLock::new(HashMap::new())),
            control_channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a WebSocket sender for a session to receive progress updates
    pub async fn register_progress_sender(
        &self,
        session_id: String,
        sender: mpsc::UnboundedSender<ProgressUpdate>,
    ) {
        let mut senders = self.progress_senders.write().await;
        let session_id_clone = session_id.clone();
        senders.insert(session_id, sender);
        tracing::info!("📡 Registered progress sender for session: {}", session_id_clone);
    }

    /// Unregister progress sender when WebSocket disconnects
    pub async fn unregister_progress_sender(&self, session_id: &str) {
        let mut senders = self.progress_senders.write().await;
        senders.remove(session_id);
        tracing::info!("📡 Unregistered progress sender for session: {}", session_id);
    }

    /// Send progress update to session's WebSocket
    pub async fn send_progress(&self, session_id: &str, update: ProgressUpdate) {
        let senders = self.progress_senders.read().await;
        if let Some(sender) = senders.get(session_id) {
            if let Err(e) = sender.send(update.clone()) {
                tracing::warn!("Failed to send progress update to session {}: {}", session_id, e);
            } else {
                tracing::info!("📤 Sent progress to session {}: {}", session_id, update.message);
            }
        } else {
            tracing::warn!("⚠️ No active WebSocket for session {}, progress not sent (message: {})", session_id, update.message);
        }
    }

    /// Create and store a new job
    pub async fn create_job(&self, job: Job) -> JobId {
        let job_id = job.id.clone();
        let mut jobs = self.jobs.write().await;
        jobs.insert(job_id.clone(), job);
        tracing::info!("🎬 Created job: {}", job_id);
        job_id
    }

    /// Get job status
    pub async fn get_job_status(&self, job_id: &str) -> Option<JobStatus> {
        let jobs = self.jobs.read().await;
        jobs.get(job_id).map(|job| job.status.clone())
    }

    /// Get job details
    pub async fn get_job(&self, job_id: &str) -> Option<Job> {
        let jobs = self.jobs.read().await;
        jobs.get(job_id).cloned()
    }

    /// Update job status
    pub async fn update_job_status(&self, job_id: &str, status: JobStatus) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.status = status.clone();

            // Update heartbeat whenever status changes (to detect stuck jobs)
            job.last_heartbeat = Utc::now();

            // Update timestamps
            match &status {
                JobStatus::Running { .. } if job.started_at.is_none() => {
                    job.started_at = Some(Utc::now());
                }
                JobStatus::Completed { .. } | JobStatus::Failed { .. } | JobStatus::Cancelled { .. } => {
                    job.completed_at = Some(Utc::now());
                }
                _ => {}
            }

            tracing::debug!("📊 Updated job {} status: {:?}", job_id, status);
        }
    }

    /// Get all jobs for a session
    pub async fn get_session_jobs(&self, session_id: &str) -> Vec<Job> {
        let jobs = self.jobs.read().await;
        jobs.values()
            .filter(|job| job.session_id == session_id)
            .cloned()
            .collect()
    }

    /// Register control channel for a job
    pub async fn register_control_channel(
        &self,
        job_id: JobId,
        sender: mpsc::UnboundedSender<JobControl>,
    ) {
        let mut channels = self.control_channels.write().await;
        channels.insert(job_id.clone(), sender);
        tracing::info!("🎛️ Registered control channel for job: {}", job_id);
    }

    /// Send control command to a job
    pub async fn send_control(&self, job_id: &str, command: JobControl) -> Result<(), String> {
        let channels = self.control_channels.read().await;
        if let Some(sender) = channels.get(job_id) {
            sender.send(command).map_err(|e| format!("Failed to send control: {}", e))?;
            Ok(())
        } else {
            Err(format!("No control channel for job {}", job_id))
        }
    }

    /// Cleanup completed/failed jobs older than specified duration
    pub async fn cleanup_old_jobs(&self, max_age_hours: i64) {
        let mut jobs = self.jobs.write().await;
        let cutoff = Utc::now() - chrono::Duration::hours(max_age_hours);

        let to_remove: Vec<JobId> = jobs
            .iter()
            .filter(|(_, job)| {
                if let Some(completed_at) = job.completed_at {
                    completed_at < cutoff
                } else {
                    false
                }
            })
            .map(|(id, _)| id.clone())
            .collect();

        for job_id in to_remove {
            jobs.remove(&job_id);
            tracing::debug!("🗑️ Cleaned up old job: {}", job_id);
        }
    }
}

impl Default for JobManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global job manager instance (to be stored in AppState)
pub type SharedJobManager = Arc<JobManager>;
