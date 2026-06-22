CREATE TABLE IF NOT EXISTS campaign_files (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    campaign_id     UUID NOT NULL REFERENCES campaigns(id) ON DELETE CASCADE,
    file_name       TEXT NOT NULL,
    r2_url          TEXT NOT NULL,
    file_type       TEXT NOT NULL CHECK (file_type IN ('image', 'video', 'document')),
    uploaded_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_campaign_files_campaign ON campaign_files(campaign_id);
