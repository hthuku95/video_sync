ALTER TABLE clipping_jobs
    ADD COLUMN IF NOT EXISTS workflow_id UUID REFERENCES app_workflows(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_clipping_jobs_workflow_id
    ON clipping_jobs (workflow_id);

ALTER TABLE manual_clipping_jobs
    ADD COLUMN IF NOT EXISTS workflow_id UUID REFERENCES app_workflows(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_manual_clipping_jobs_workflow_id
    ON manual_clipping_jobs (workflow_id);
