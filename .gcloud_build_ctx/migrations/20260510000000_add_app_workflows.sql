CREATE TABLE IF NOT EXISTS app_workflows (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'planning', 'running', 'waiting_for_input', 'waiting_for_external_service', 'retrying', 'completed', 'failed', 'cancelled')),
    session_uuid TEXT,
    user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    source_table TEXT,
    source_record_id UUID,
    request_summary TEXT NOT NULL,
    current_step TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    artifact_requirements JSONB NOT NULL DEFAULT '[]'::jsonb,
    artifact_status JSONB NOT NULL DEFAULT '{}'::jsonb,
    result_summary TEXT,
    error_message TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_app_workflows_status_created
    ON app_workflows (status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_app_workflows_session_uuid
    ON app_workflows (session_uuid, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_app_workflows_user_created
    ON app_workflows (user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_app_workflows_source
    ON app_workflows (source_table, source_record_id);

CREATE TABLE IF NOT EXISTS app_workflow_events (
    id BIGSERIAL PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES app_workflows(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    node_name TEXT,
    message TEXT NOT NULL,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_app_workflow_events_workflow_created
    ON app_workflow_events (workflow_id, created_at DESC);

ALTER TABLE agent_background_jobs
    ADD COLUMN IF NOT EXISTS workflow_id UUID REFERENCES app_workflows(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_agent_background_jobs_workflow_id
    ON agent_background_jobs (workflow_id);

ALTER TABLE service_sample_requests
    ADD COLUMN IF NOT EXISTS workflow_id UUID REFERENCES app_workflows(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_service_sample_requests_workflow_id
    ON service_sample_requests (workflow_id);

CREATE OR REPLACE FUNCTION update_app_workflows_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_app_workflows_updated_at ON app_workflows;

CREATE TRIGGER trg_app_workflows_updated_at
    BEFORE UPDATE ON app_workflows
    FOR EACH ROW EXECUTE FUNCTION update_app_workflows_updated_at();
