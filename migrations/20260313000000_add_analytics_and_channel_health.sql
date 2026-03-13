-- Analytics Sync Log + Channel Health Scoring (Mar 13, 2026)

-- Table: analytics_sync_log
-- Tracks each run of the 6-hourly analytics sync job for monitoring
CREATE TABLE IF NOT EXISTS analytics_sync_log (
    id SERIAL PRIMARY KEY,
    clips_synced INTEGER NOT NULL DEFAULT 0,
    clips_failed INTEGER NOT NULL DEFAULT 0,
    duration_seconds INTEGER NOT NULL DEFAULT 0,
    sync_completed_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_analytics_sync_log_completed ON analytics_sync_log(sync_completed_at DESC);

-- Table: source_channel_health
-- Tracks per-source-channel success/failure rate so the system can
-- deprioritize channels that consistently fail (private videos, geo-blocks, etc.)
CREATE TABLE IF NOT EXISTS source_channel_health (
    id SERIAL PRIMARY KEY,
    source_channel_id INTEGER NOT NULL REFERENCES youtube_source_channels(id) ON DELETE CASCADE,

    -- Rolling counters (last 30 days)
    jobs_attempted INTEGER NOT NULL DEFAULT 0,
    jobs_succeeded INTEGER NOT NULL DEFAULT 0,   -- reached 'completed' status
    jobs_failed_permanent INTEGER NOT NULL DEFAULT 0,  -- private/unavailable/forbidden
    jobs_failed_transient INTEGER NOT NULL DEFAULT 0,  -- network/quota errors

    -- Derived score: 0.0 (always fails) → 1.0 (always succeeds)
    health_score DECIMAL(4,3) NOT NULL DEFAULT 1.0,

    -- Last error seen on this channel
    last_error TEXT,
    last_error_at TIMESTAMPTZ,

    -- When health was last recalculated
    last_calculated_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),

    UNIQUE(source_channel_id)
);

CREATE INDEX IF NOT EXISTS idx_source_channel_health_score ON source_channel_health(health_score DESC);

-- Function: recalculate health score for a source channel
-- health_score = succeeded / attempted (capped at 1.0, 0.0 if never attempted)
CREATE OR REPLACE FUNCTION recalculate_channel_health(p_source_channel_id INTEGER)
RETURNS void AS $$
DECLARE
    v_attempted INTEGER;
    v_succeeded INTEGER;
    v_score DECIMAL(4,3);
BEGIN
    SELECT
        COUNT(*) as attempted,
        COUNT(*) FILTER (WHERE cj.status = 'completed') as succeeded
    INTO v_attempted, v_succeeded
    FROM clipping_jobs cj
    JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
    WHERE ycl.source_channel_id = p_source_channel_id
      AND cj.created_at > NOW() - INTERVAL '30 days';

    IF v_attempted = 0 THEN
        v_score := 1.0;  -- No data: assume healthy
    ELSE
        v_score := LEAST(1.0, GREATEST(0.0, v_succeeded::DECIMAL / v_attempted::DECIMAL));
    END IF;

    INSERT INTO source_channel_health (source_channel_id, jobs_attempted, jobs_succeeded, health_score, last_calculated_at, updated_at)
    VALUES (p_source_channel_id, v_attempted, v_succeeded, v_score, NOW(), NOW())
    ON CONFLICT (source_channel_id) DO UPDATE
        SET jobs_attempted = EXCLUDED.jobs_attempted,
            jobs_succeeded = EXCLUDED.jobs_succeeded,
            health_score = EXCLUDED.health_score,
            last_calculated_at = EXCLUDED.last_calculated_at,
            updated_at = EXCLUDED.updated_at;
END;
$$ LANGUAGE plpgsql;

-- Seed health scores for existing source channels
INSERT INTO source_channel_health (source_channel_id, health_score)
SELECT id, 1.0 FROM youtube_source_channels
ON CONFLICT (source_channel_id) DO NOTHING;
