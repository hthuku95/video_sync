CREATE TABLE IF NOT EXISTS app_workflow_nodes (
    id BIGSERIAL PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES app_workflows(id) ON DELETE CASCADE,
    node_key TEXT NOT NULL,
    node_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'waiting', 'completed', 'failed', 'skipped')),
    attempt_count INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    input JSONB NOT NULL DEFAULT '{}'::jsonb,
    output JSONB NOT NULL DEFAULT '{}'::jsonb,
    error_message TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    last_heartbeat_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (workflow_id, node_key)
);

CREATE INDEX IF NOT EXISTS idx_app_workflow_nodes_workflow_status
    ON app_workflow_nodes (workflow_id, status, node_key);

CREATE INDEX IF NOT EXISTS idx_app_workflow_nodes_stale_running
    ON app_workflow_nodes (status, last_heartbeat_at)
    WHERE status IN ('running', 'waiting');

CREATE OR REPLACE FUNCTION update_app_workflow_nodes_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_app_workflow_nodes_updated_at ON app_workflow_nodes;

CREATE TRIGGER trg_app_workflow_nodes_updated_at
    BEFORE UPDATE ON app_workflow_nodes
    FOR EACH ROW EXECUTE FUNCTION update_app_workflow_nodes_updated_at();
