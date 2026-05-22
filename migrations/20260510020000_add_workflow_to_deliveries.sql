ALTER TABLE deliveries
    ADD COLUMN IF NOT EXISTS workflow_id UUID REFERENCES app_workflows(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_deliveries_workflow_id
    ON deliveries (workflow_id);
