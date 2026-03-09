-- Heartbeat column: workers touch this every 30s during long-running phases.
-- Stuck detection uses COALESCE(worker_heartbeat_at, updated_at) so existing
-- jobs without heartbeats still work correctly via updated_at fallback.
ALTER TABLE clipping_jobs
    ADD COLUMN IF NOT EXISTS worker_heartbeat_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS heartbeat_data JSONB;

-- Worker process liveness table: one row per worker process.
-- Health endpoint reads last_seen_at to determine if worker is alive.
CREATE TABLE IF NOT EXISTS worker_heartbeats (
    worker_id        TEXT PRIMARY KEY,
    last_seen_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    jobs_processed   INT NOT NULL DEFAULT 0,
    jobs_failed      INT NOT NULL DEFAULT 0,
    current_job_id   INT REFERENCES clipping_jobs(id) ON DELETE SET NULL,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Add 'discarded' status — permanently exhausted jobs that need admin review.
-- Distinct from 'failed' (retryable) vs 'discarded' (gave up, DLQ).
ALTER TABLE clipping_jobs
    DROP CONSTRAINT IF EXISTS valid_status_values;

ALTER TABLE clipping_jobs
    ADD CONSTRAINT valid_status_values CHECK (
        status::text = ANY (ARRAY[
            'pending',
            'analyzing',
            'analyzed',
            'no_clips_found',
            'downloading',
            'downloaded',
            'extracting_clips',
            'clips_extracted',
            'vectorizing',
            'vectorized',
            'posting',
            'completed',
            'failed',
            'cancelled',
            'discarded'
        ]::text[])
    );

-- Performance index for heartbeat-based stuck detection
CREATE INDEX IF NOT EXISTS idx_clipping_jobs_heartbeat
    ON clipping_jobs (worker_heartbeat_at)
    WHERE status IN ('downloading', 'analyzing', 'extracting_clips', 'posting');

-- Performance index for pending-too-long detection
CREATE INDEX IF NOT EXISTS idx_clipping_jobs_pending_unclaimed
    ON clipping_jobs (created_at)
    WHERE status = 'pending' AND claimed_by IS NULL;
