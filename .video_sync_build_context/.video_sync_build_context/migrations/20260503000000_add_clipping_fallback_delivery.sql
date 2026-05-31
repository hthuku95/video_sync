-- Link automatic clipping jobs to a generated longform fallback delivery.
-- When source download fails, the system can still produce one narrated
-- explainer render and keep the relationship visible from the clipping job.

ALTER TABLE clipping_jobs
    ADD COLUMN IF NOT EXISTS fallback_delivery_id UUID
        REFERENCES deliveries(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS fallback_strategy TEXT,
    ADD COLUMN IF NOT EXISTS fallback_activated_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_clipping_jobs_fallback_delivery
    ON clipping_jobs(fallback_delivery_id)
    WHERE fallback_delivery_id IS NOT NULL;
