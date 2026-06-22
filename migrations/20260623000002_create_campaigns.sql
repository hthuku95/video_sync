CREATE TABLE IF NOT EXISTS campaigns (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id             INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    service_type        TEXT NOT NULL CHECK (service_type IN ('clipping', 'education')),
    brief               TEXT NOT NULL,
    style               TEXT NOT NULL DEFAULT 'cinematic',
    duration            FLOAT8 NOT NULL DEFAULT 30.0,
    schedule            JSONB NOT NULL DEFAULT '[]'::jsonb,
    platforms           JSONB NOT NULL DEFAULT '[]'::jsonb,
    posts_per_day       INTEGER NOT NULL DEFAULT 3,
    start_date          TIMESTAMPTZ NOT NULL,
    end_date            TIMESTAMPTZ NOT NULL,
    zernio_profile_id   TEXT,
    status              TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'completed', 'cancelled')),
    total_posts_planned INTEGER NOT NULL DEFAULT 0,
    total_posts_published INTEGER NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_campaigns_user_id ON campaigns(user_id);
CREATE INDEX IF NOT EXISTS idx_campaigns_status ON campaigns(status);
CREATE INDEX IF NOT EXISTS idx_campaigns_active_dates ON campaigns(status, start_date, end_date);
