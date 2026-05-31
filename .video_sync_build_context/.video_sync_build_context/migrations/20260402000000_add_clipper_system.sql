-- Clipper role on users table
ALTER TABLE users ADD COLUMN IF NOT EXISTS is_clipper BOOLEAN NOT NULL DEFAULT false;

-- Invite tokens for clipper signup
CREATE TABLE IF NOT EXISTS clipper_invite_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    token VARCHAR(64) UNIQUE NOT NULL,
    label TEXT,
    created_by_admin_id INTEGER REFERENCES users(id),
    used_by_user_id INTEGER REFERENCES users(id),
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Manual clipping jobs (no linkage_id required)
CREATE TABLE IF NOT EXISTS manual_clipping_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id INTEGER NOT NULL REFERENCES users(id),
    video_url TEXT NOT NULL,
    video_platform TEXT NOT NULL,
    video_title TEXT,
    video_duration_seconds FLOAT8,
    clips_requested INTEGER NOT NULL DEFAULT 3,
    min_clip_duration_seconds INTEGER NOT NULL DEFAULT 30,
    max_clip_duration_seconds INTEGER NOT NULL DEFAULT 120,
    status TEXT NOT NULL DEFAULT 'pending',
    progress_percent INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    viral_moments_json JSONB,
    clips_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ
);

-- Clips produced by manual clipping jobs
CREATE TABLE IF NOT EXISTS manual_clipping_clips (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id UUID NOT NULL REFERENCES manual_clipping_jobs(id) ON DELETE CASCADE,
    clip_number INTEGER NOT NULL,
    title TEXT,
    description TEXT,
    start_time_seconds FLOAT8,
    end_time_seconds FLOAT8,
    duration_seconds FLOAT8,
    quality_score FLOAT8,
    viral_factors JSONB,
    r2_clip_key TEXT,
    r2_clip_url TEXT,
    r2_clip_url_expires_at TIMESTAMPTZ,
    thumbnail_r2_key TEXT,
    thumbnail_r2_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Prospects discovered by the admin prospect finder tool
CREATE TABLE IF NOT EXISTS prospects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    platform TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    platform_url TEXT NOT NULL,
    subscriber_count BIGINT,
    avg_viewer_count BIGINT,
    last_active_at TIMESTAMPTZ,
    content_category TEXT,
    channel_description TEXT,
    prospect_type TEXT NOT NULL,
    ai_score FLOAT8,
    ai_reasoning TEXT,
    dm_script_creator TEXT,
    dm_script_clipper TEXT,
    contact_status TEXT NOT NULL DEFAULT 'new',
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(platform, channel_id)
);
