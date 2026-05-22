CREATE TABLE IF NOT EXISTS service_sample_requests (
    id UUID PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    service_slug TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'videosync_service',
    reference_url TEXT,
    prospect_name TEXT,
    brief TEXT NOT NULL,
    generated_prompt TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_service_sample_requests_user_created
    ON service_sample_requests (user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_service_sample_requests_source
    ON service_sample_requests (source, service_slug);
