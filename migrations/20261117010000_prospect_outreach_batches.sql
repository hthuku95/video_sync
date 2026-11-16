-- Prospect outreach batches (owner design Nov 16 2026): sample-first outreach.
-- Clicking "Send Bulk Email" creates a durable batch; for each prospect
-- WITHOUT a recent sample, a sample video is generated first (background
-- lane); the email is sent ONLY when the sample completes, carrying the
-- sample link. Durable = restarts never lose the batch; per-prospect tracking.
CREATE TABLE IF NOT EXISTS prospect_outreach_batches (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    created_by INTEGER NOT NULL REFERENCES users(id),
    -- the filters used (audit)
    service_type VARCHAR(64),
    fresh_days INTEGER,
    include_contacted BOOLEAN NOT NULL DEFAULT FALSE,
    status VARCHAR(24) NOT NULL DEFAULT 'processing', -- processing | completed
    total_items INTEGER NOT NULL DEFAULT 0,
    emails_sent INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS prospect_outreach_items (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    batch_id UUID NOT NULL REFERENCES prospect_outreach_batches(id) ON DELETE CASCADE,
    prospect_id UUID NOT NULL REFERENCES prospects(id) ON DELETE CASCADE,
    prospect_name VARCHAR(255) NOT NULL DEFAULT '',
    prospect_email VARCHAR(255) NOT NULL DEFAULT '',
    -- pending_sample | sample_generating | ready_to_send | email_sent | email_failed | skipped
    status VARCHAR(24) NOT NULL DEFAULT 'pending_sample',
    -- The sample delivery (created at batch time when needed)
    delivery_id UUID,
    sample_url TEXT,
    email_log_id BIGINT,
    error_message TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (batch_id, prospect_id)
);
CREATE INDEX IF NOT EXISTS idx_poi_batch_status ON prospect_outreach_items(batch_id, status);
