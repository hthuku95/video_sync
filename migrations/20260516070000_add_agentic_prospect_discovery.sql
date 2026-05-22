-- Agentic prospect discovery: durable LangGraph-style run/checkpoint state
-- plus Telegram public-channel discovery candidates from MTProto user search.

CREATE TABLE IF NOT EXISTS prospect_agent_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    run_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running',
    goal TEXT NOT NULL,
    input JSONB NOT NULL DEFAULT '{}'::jsonb,
    state JSONB NOT NULL DEFAULT '{}'::jsonb,
    current_step TEXT NOT NULL DEFAULT 'created',
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS prospect_agent_checkpoints (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID NOT NULL REFERENCES prospect_agent_runs(id) ON DELETE CASCADE,
    checkpoint_num INTEGER NOT NULL,
    step TEXT NOT NULL,
    state JSONB NOT NULL DEFAULT '{}'::jsonb,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(run_id, checkpoint_num)
);

CREATE TABLE IF NOT EXISTS telegram_discovered_channels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID REFERENCES prospect_agent_runs(id) ON DELETE SET NULL,
    query TEXT NOT NULL,
    channel_id BIGINT,
    username TEXT,
    title TEXT NOT NULL,
    is_broadcast BOOLEAN NOT NULL DEFAULT FALSE,
    is_megagroup BOOLEAN NOT NULL DEFAULT FALSE,
    participants_count INTEGER,
    score INTEGER,
    score_reason TEXT,
    service_type TEXT,
    status TEXT NOT NULL DEFAULT 'new',
    source TEXT NOT NULL DEFAULT 'mtproto_contacts_search',
    raw JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_prospect_agent_runs_status_created
    ON prospect_agent_runs(status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_prospect_agent_runs_type_created
    ON prospect_agent_runs(run_type, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_prospect_agent_checkpoints_run_created
    ON prospect_agent_checkpoints(run_id, checkpoint_num DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_telegram_discovered_channels_username
    ON telegram_discovered_channels(LOWER(username))
    WHERE username IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_telegram_discovered_channels_channel_id
    ON telegram_discovered_channels(channel_id)
    WHERE channel_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_telegram_discovered_channels_score
    ON telegram_discovered_channels(status, score DESC NULLS LAST, created_at DESC);
