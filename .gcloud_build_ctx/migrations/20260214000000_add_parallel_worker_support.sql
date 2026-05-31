-- Add worker coordination columns for parallel job processing
-- This migration enables atomic job claiming using PostgreSQL FOR UPDATE SKIP LOCKED

ALTER TABLE clipping_jobs
ADD COLUMN IF NOT EXISTS claimed_by VARCHAR(100),
ADD COLUMN IF NOT EXISTS claimed_at TIMESTAMPTZ,
ADD COLUMN IF NOT EXISTS last_processed_by VARCHAR(100);

-- Index for fast unclaimed job lookup (critical for performance)
CREATE INDEX IF NOT EXISTS idx_clipping_jobs_pending_unclaimed
ON clipping_jobs(status, created_at)
WHERE status = 'pending' AND claimed_by IS NULL;

-- Composite index for worker coordination (ensures efficient claiming queries)
CREATE INDEX IF NOT EXISTS idx_clipping_jobs_worker_status
ON clipping_jobs(claimed_by, status, updated_at);

-- Add comments for documentation
COMMENT ON COLUMN clipping_jobs.claimed_by IS
'Worker instance ID that claimed this job (format: hostname-pid-timestamp). Used for parallel worker coordination to prevent duplicate processing.';

COMMENT ON COLUMN clipping_jobs.claimed_at IS
'Timestamp when the job was claimed by a worker. Used for detecting stuck workers.';

COMMENT ON COLUMN clipping_jobs.last_processed_by IS
'Worker instance ID that last processed this job. Useful for debugging and monitoring worker performance.';

COMMENT ON INDEX idx_clipping_jobs_pending_unclaimed IS
'Optimizes atomic job claiming queries using FOR UPDATE SKIP LOCKED. Critical for parallel worker performance.';

-- Success message
DO $$
BEGIN
    RAISE NOTICE '✅ Parallel worker support migration completed successfully';
    RAISE NOTICE 'Added columns: claimed_by, claimed_at, last_processed_by';
    RAISE NOTICE 'Created indexes: idx_clipping_jobs_pending_unclaimed, idx_clipping_jobs_worker_status';
END $$;
