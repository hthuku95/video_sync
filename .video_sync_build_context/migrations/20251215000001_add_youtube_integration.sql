-- Add Google OAuth and YouTube channel integration
-- Migration: 20251215000001_add_youtube_integration.sql

-- ============================================================================
-- PART 1: Update users table for Google OAuth
-- ============================================================================

-- Add Google OAuth fields to users table
ALTER TABLE users ADD COLUMN IF NOT EXISTS google_id VARCHAR(255) UNIQUE;
ALTER TABLE users ADD COLUMN IF NOT EXISTS google_email VARCHAR(255);
ALTER TABLE users ADD COLUMN IF NOT EXISTS google_picture TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS google_access_token TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS google_refresh_token TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS google_token_expiry TIMESTAMPTZ;

-- Create index for Google ID lookups
CREATE INDEX IF NOT EXISTS idx_users_google_id ON users(google_id);

-- ============================================================================
-- PART 2: Create YouTube channels connection table
-- ============================================================================

CREATE TABLE IF NOT EXISTS connected_youtube_channels (
    id SERIAL PRIMARY KEY,

    -- User relationship
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    -- YouTube channel details
    channel_id VARCHAR(255) NOT NULL,
    channel_name VARCHAR(255) NOT NULL,
    channel_description TEXT,
    channel_thumbnail_url TEXT,
    subscriber_count BIGINT DEFAULT 0,
    video_count BIGINT DEFAULT 0,

    -- OAuth tokens (encrypted in production)
    access_token TEXT NOT NULL,
    refresh_token TEXT NOT NULL,
    token_expiry TIMESTAMPTZ NOT NULL,

    -- Scopes granted
    granted_scopes TEXT NOT NULL, -- Comma-separated list of scopes

    -- Status
    is_active BOOLEAN DEFAULT true,
    last_sync_at TIMESTAMPTZ,

    -- Timestamps
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),

    -- Ensure one channel can only be connected once per user
    UNIQUE(user_id, channel_id)
);

-- Create indexes for fast queries
CREATE INDEX IF NOT EXISTS idx_youtube_channels_user_id ON connected_youtube_channels(user_id);
CREATE INDEX IF NOT EXISTS idx_youtube_channels_channel_id ON connected_youtube_channels(channel_id);
CREATE INDEX IF NOT EXISTS idx_youtube_channels_active ON connected_youtube_channels(is_active) WHERE is_active = true;

-- ============================================================================
-- PART 3: Create YouTube upload history table
-- ============================================================================

CREATE TABLE IF NOT EXISTS youtube_uploads (
    id SERIAL PRIMARY KEY,

    -- Relationships
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id INTEGER NOT NULL REFERENCES connected_youtube_channels(id) ON DELETE CASCADE,
    session_id INTEGER REFERENCES chat_sessions(id) ON DELETE SET NULL,

    -- Local video file
    local_video_path TEXT NOT NULL,
    local_file_id VARCHAR(255),

    -- YouTube video details
    youtube_video_id VARCHAR(255) UNIQUE,
    video_title VARCHAR(255) NOT NULL,
    video_description TEXT,
    video_category VARCHAR(50) DEFAULT '22', -- People & Blogs
    privacy_status VARCHAR(20) DEFAULT 'private', -- public, private, unlisted

    -- Upload status
    upload_status VARCHAR(50) DEFAULT 'pending', -- pending, uploading, completed, failed
    upload_progress INTEGER DEFAULT 0, -- 0-100
    error_message TEXT,

    -- YouTube response
    youtube_url TEXT,
    published_at TIMESTAMPTZ,

    -- Timestamps
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Create indexes
CREATE INDEX IF NOT EXISTS idx_youtube_uploads_user_id ON youtube_uploads(user_id);
CREATE INDEX IF NOT EXISTS idx_youtube_uploads_channel_id ON youtube_uploads(channel_id);
CREATE INDEX IF NOT EXISTS idx_youtube_uploads_status ON youtube_uploads(upload_status);
CREATE INDEX IF NOT EXISTS idx_youtube_uploads_youtube_id ON youtube_uploads(youtube_video_id);

-- ============================================================================
-- PART 4: Add comments and triggers
-- ============================================================================

COMMENT ON TABLE connected_youtube_channels IS 'Stores user-connected YouTube channels with OAuth tokens for video uploads';
COMMENT ON TABLE youtube_uploads IS 'Tracks video uploads to YouTube channels';

COMMENT ON COLUMN users.google_id IS 'Google OAuth subject identifier (sub claim)';
COMMENT ON COLUMN connected_youtube_channels.granted_scopes IS 'OAuth scopes: youtube.upload, youtube.readonly, etc.';

-- Add update trigger for connected_youtube_channels
CREATE TRIGGER update_youtube_channels_updated_at
BEFORE UPDATE ON connected_youtube_channels
FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Add update trigger for youtube_uploads
CREATE TRIGGER update_youtube_uploads_updated_at
BEFORE UPDATE ON youtube_uploads
FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
