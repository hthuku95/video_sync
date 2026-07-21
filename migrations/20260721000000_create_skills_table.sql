-- Skills table: stores reusable patterns from successful workflows and user corrections
-- Enables the agent to learn from experience per-campaign and per-service-type

CREATE TABLE IF NOT EXISTS skills (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    service_type TEXT,
    campaign_id UUID,
    name TEXT NOT NULL,
    description TEXT,
    trigger_conditions JSONB DEFAULT '{}'::jsonb,
    tool_sequence JSONB DEFAULT '[]'::jsonb,
    source TEXT NOT NULL DEFAULT 'successful_workflow' CHECK (source IN ('successful_workflow', 'user_correction', 'manual')),
    correction JSONB DEFAULT NULL,
    success_count INT NOT NULL DEFAULT 1,
    scope TEXT NOT NULL DEFAULT 'campaign' CHECK (scope IN ('campaign', 'service', 'global')),
    restricted_to_user_id INT,
    qdrant_point_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_skills_user_id ON skills(user_id);
CREATE INDEX IF NOT EXISTS idx_skills_service_type ON skills(service_type);
CREATE INDEX IF NOT EXISTS idx_skills_campaign_id ON skills(campaign_id);
CREATE INDEX IF NOT EXISTS idx_skills_scope ON skills(scope);
CREATE INDEX IF NOT EXISTS idx_skills_source ON skills(source);
