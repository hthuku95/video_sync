-- Agent background jobs: tracks AI agent tasks so they survive WebSocket disconnects
CREATE TABLE IF NOT EXISTS agent_background_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_uuid TEXT NOT NULL,
    session_id INTEGER REFERENCES chat_sessions(id) ON DELETE CASCADE,
    user_message TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'completed', 'failed')),
    result TEXT,
    error TEXT,
    progress_log JSONB NOT NULL DEFAULT '[]',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_agent_jobs_session_uuid ON agent_background_jobs(session_uuid);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_status ON agent_background_jobs(status);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_created ON agent_background_jobs(created_at DESC);

-- Auto-update updated_at
CREATE OR REPLACE FUNCTION update_agent_jobs_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_agent_jobs_updated_at
    BEFORE UPDATE ON agent_background_jobs
    FOR EACH ROW EXECUTE FUNCTION update_agent_jobs_updated_at();
