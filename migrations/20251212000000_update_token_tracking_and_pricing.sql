-- Update model pricing to current 2025 rates and create comprehensive token usage tracking
-- Migration: 20251212000000_update_token_tracking_and_pricing.sql

-- ============================================================================
-- PART 1: Create api_token_usage table for detailed per-request tracking
-- ============================================================================

CREATE TABLE IF NOT EXISTS api_token_usage (
    id SERIAL PRIMARY KEY,

    -- Relationships
    session_id INTEGER NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    message_id INTEGER REFERENCES conversation_messages(id) ON DELETE SET NULL,
    job_id TEXT,  -- For background jobs

    -- API Details
    provider TEXT NOT NULL CHECK (provider IN ('claude', 'gemini', 'elevenlabs', 'pexels')),
    model TEXT NOT NULL,
    request_type TEXT NOT NULL,  -- 'chat', 'background_job', 'tool_call', etc.

    -- Token Usage
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER GENERATED ALWAYS AS (input_tokens + output_tokens) STORED,

    -- Cost (in USD cents to avoid floating point precision issues)
    input_cost_cents BIGINT NOT NULL DEFAULT 0,
    output_cost_cents BIGINT NOT NULL DEFAULT 0,
    total_cost_cents BIGINT GENERATED ALWAYS AS (input_cost_cents + output_cost_cents) STORED,

    -- Additional metadata (for Claude prompt caching, etc.)
    cache_creation_tokens INTEGER DEFAULT 0,
    cache_read_tokens INTEGER DEFAULT 0,
    context_size INTEGER DEFAULT 0,

    -- Timestamps
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Indexes for fast queries
CREATE INDEX IF NOT EXISTS idx_token_usage_session ON api_token_usage(session_id);
CREATE INDEX IF NOT EXISTS idx_token_usage_user ON api_token_usage(user_id);
CREATE INDEX IF NOT EXISTS idx_token_usage_created ON api_token_usage(created_at);
CREATE INDEX IF NOT EXISTS idx_token_usage_provider ON api_token_usage(provider);
CREATE INDEX IF NOT EXISTS idx_token_usage_model ON api_token_usage(model);
CREATE INDEX IF NOT EXISTS idx_token_usage_job ON api_token_usage(job_id) WHERE job_id IS NOT NULL;

-- ============================================================================
-- PART 2: Update model_pricing with current 2025 rates
-- ============================================================================

-- Update Claude Sonnet 4.5 pricing (your current model)
INSERT INTO system_settings (setting_key, setting_value, setting_type, description)
VALUES
    ('model_pricing.claude-sonnet-4-5.input_base', '3.00', 'decimal', 'Claude Sonnet 4.5: Input cost per 1M tokens (≤200K context)'),
    ('model_pricing.claude-sonnet-4-5.input_extended', '6.00', 'decimal', 'Claude Sonnet 4.5: Input cost per 1M tokens (>200K context)'),
    ('model_pricing.claude-sonnet-4-5.output_base', '15.00', 'decimal', 'Claude Sonnet 4.5: Output cost per 1M tokens (≤200K context)'),
    ('model_pricing.claude-sonnet-4-5.output_extended', '22.50', 'decimal', 'Claude Sonnet 4.5: Output cost per 1M tokens (>200K context)'),
    ('model_pricing.claude-sonnet-4-5.last_updated', '2025-12-12', 'string', 'Last pricing update date')
ON CONFLICT (setting_key) DO UPDATE
SET setting_value = EXCLUDED.setting_value, updated_at = NOW();

-- Update Claude Sonnet 3.5 pricing
INSERT INTO system_settings (setting_key, setting_value, setting_type, description)
VALUES
    ('model_pricing.claude-3-5-sonnet.input', '3.00', 'decimal', 'Claude Sonnet 3.5: Input cost per 1M tokens'),
    ('model_pricing.claude-3-5-sonnet.output', '15.00', 'decimal', 'Claude Sonnet 3.5: Output cost per 1M tokens'),
    ('model_pricing.claude-3-5-sonnet.last_updated', '2025-12-12', 'string', 'Last pricing update date')
ON CONFLICT (setting_key) DO UPDATE
SET setting_value = EXCLUDED.setting_value, updated_at = NOW();

-- Add Gemini 2.0 Flash pricing (your current model)
INSERT INTO system_settings (setting_key, setting_value, setting_type, description)
VALUES
    ('model_pricing.gemini-2.0-flash.input', '0.10', 'decimal', 'Gemini 2.0 Flash: Input cost per 1M tokens'),
    ('model_pricing.gemini-2.0-flash.output', '0.40', 'decimal', 'Gemini 2.0 Flash: Output cost per 1M tokens'),
    ('model_pricing.gemini-2.0-flash.last_updated', '2025-12-12', 'string', 'Last pricing update date')
ON CONFLICT (setting_key) DO UPDATE
SET setting_value = EXCLUDED.setting_value, updated_at = NOW();

-- Add Gemini 2.5 Flash pricing
INSERT INTO system_settings (setting_key, setting_value, setting_type, description)
VALUES
    ('model_pricing.gemini-2.5-flash.input', '0.30', 'decimal', 'Gemini 2.5 Flash: Input cost per 1M tokens'),
    ('model_pricing.gemini-2.5-flash.output', '2.50', 'decimal', 'Gemini 2.5 Flash: Output cost per 1M tokens'),
    ('model_pricing.gemini-2.5-flash.last_updated', '2025-12-12', 'string', 'Last pricing update date')
ON CONFLICT (setting_key) DO UPDATE
SET setting_value = EXCLUDED.setting_value, updated_at = NOW();

-- ============================================================================
-- PART 3: Create useful views for reporting
-- ============================================================================

-- User usage summary view
CREATE OR REPLACE VIEW user_usage_summary AS
SELECT
    u.id as user_id,
    u.username,
    u.email,
    COUNT(DISTINCT t.id) as total_api_calls,
    COALESCE(SUM(t.input_tokens), 0) as total_input_tokens,
    COALESCE(SUM(t.output_tokens), 0) as total_output_tokens,
    COALESCE(SUM(t.total_tokens), 0) as total_tokens,
    COALESCE(SUM(t.total_cost_cents), 0)::FLOAT / 100 as total_cost_usd,
    COUNT(DISTINCT t.session_id) as sessions_with_usage,
    MAX(t.created_at) as last_api_call_at
FROM users u
LEFT JOIN chat_sessions cs ON cs.user_id = u.id
LEFT JOIN api_token_usage t ON t.session_id = cs.id
GROUP BY u.id, u.username, u.email;

-- Session usage summary view
CREATE OR REPLACE VIEW session_usage_summary AS
SELECT
    cs.id as session_id,
    cs.session_uuid,
    cs.user_id,
    u.username,
    COUNT(DISTINCT t.id) as total_api_calls,
    COALESCE(SUM(t.input_tokens), 0) as total_input_tokens,
    COALESCE(SUM(t.output_tokens), 0) as total_output_tokens,
    COALESCE(SUM(t.total_tokens), 0) as total_tokens,
    COALESCE(SUM(t.total_cost_cents), 0)::FLOAT / 100 as total_cost_usd,
    cs.created_at as session_started,
    MAX(t.created_at) as last_api_call_at
FROM chat_sessions cs
LEFT JOIN users u ON u.id = cs.user_id
LEFT JOIN api_token_usage t ON t.session_id = cs.id
GROUP BY cs.id, cs.session_uuid, cs.user_id, u.username, cs.created_at;

-- Provider comparison view
CREATE OR REPLACE VIEW provider_usage_summary AS
SELECT
    provider,
    model,
    COUNT(*) as total_requests,
    SUM(input_tokens) as total_input_tokens,
    SUM(output_tokens) as total_output_tokens,
    SUM(total_tokens) as total_tokens,
    SUM(total_cost_cents)::FLOAT / 100 as total_cost_usd,
    AVG(input_tokens)::INTEGER as avg_input_tokens,
    AVG(output_tokens)::INTEGER as avg_output_tokens,
    MIN(created_at) as first_used,
    MAX(created_at) as last_used
FROM api_token_usage
GROUP BY provider, model;

-- Daily usage summary view
CREATE OR REPLACE VIEW daily_usage_summary AS
SELECT
    DATE(created_at) as date,
    provider,
    model,
    COUNT(*) as total_requests,
    SUM(input_tokens) as total_input_tokens,
    SUM(output_tokens) as total_output_tokens,
    SUM(total_cost_cents)::FLOAT / 100 as total_cost_usd
FROM api_token_usage
GROUP BY DATE(created_at), provider, model
ORDER BY date DESC, total_cost_usd DESC;

-- ============================================================================
-- PART 4: Add helpful comments for admin dashboard
-- ============================================================================

COMMENT ON TABLE api_token_usage IS 'Tracks API token usage and costs per request for billing and analytics';
COMMENT ON COLUMN api_token_usage.provider IS 'API provider: claude, gemini, elevenlabs, pexels';
COMMENT ON COLUMN api_token_usage.input_cost_cents IS 'Input cost in USD cents (e.g., 150 = $1.50)';
COMMENT ON COLUMN api_token_usage.total_cost_cents IS 'Total cost in USD cents (computed column)';

COMMENT ON TABLE system_settings IS 'System-wide settings including model pricing (editable via admin dashboard)';
COMMENT ON VIEW user_usage_summary IS 'Per-user API usage and cost summary for billing';
COMMENT ON VIEW session_usage_summary IS 'Per-session API usage and cost summary';
COMMENT ON VIEW provider_usage_summary IS 'Compare usage across different AI providers';
COMMENT ON VIEW daily_usage_summary IS 'Daily breakdown of API usage and costs';
