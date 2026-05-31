ALTER TABLE clipping_jobs
    ADD COLUMN IF NOT EXISTS supervisor_status TEXT NOT NULL DEFAULT 'healthy',
    ADD COLUMN IF NOT EXISTS supervisor_reason TEXT,
    ADD COLUMN IF NOT EXISTS supervisor_last_action TEXT,
    ADD COLUMN IF NOT EXISTS supervisor_last_run_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS blocked_by_job_id INTEGER REFERENCES clipping_jobs(id) ON DELETE SET NULL;

CREATE TABLE IF NOT EXISTS clipping_supervisor_events (
    id BIGSERIAL PRIMARY KEY,
    clipping_job_id INTEGER NOT NULL REFERENCES clipping_jobs(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    message TEXT NOT NULL,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_clipping_jobs_one_active_per_source
    ON clipping_jobs (linkage_id, source_video_id)
    WHERE status IN ('pending', 'downloading', 'analyzing', 'extracting_clips', 'posting');
