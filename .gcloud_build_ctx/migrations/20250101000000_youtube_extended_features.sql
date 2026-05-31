-- YouTube Extended Features Migration
-- Adds support for: video deletion, metadata editing, thumbnails, playlists,
-- analytics, comments, captions, scheduling, and resumable uploads

-- ============================================================================
-- PHASE 1: Core Video Management
-- ============================================================================

-- Add columns to youtube_uploads table for extended video management
ALTER TABLE youtube_uploads
ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ,
ADD COLUMN IF NOT EXISTS metadata_updated_at TIMESTAMPTZ,
ADD COLUMN IF NOT EXISTS custom_thumbnail_path TEXT,
ADD COLUMN IF NOT EXISTS scheduled_publish_at TIMESTAMPTZ,
ADD COLUMN IF NOT EXISTS is_scheduled BOOLEAN DEFAULT false,
ADD COLUMN IF NOT EXISTS upload_session_url TEXT,
ADD COLUMN IF NOT EXISTS bytes_uploaded BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_bytes BIGINT,
ADD COLUMN IF NOT EXISTS is_resumable BOOLEAN DEFAULT false;

-- Add OAuth scope migration flag to channels
ALTER TABLE connected_youtube_channels
ADD COLUMN IF NOT EXISTS requires_reauth BOOLEAN DEFAULT false;

-- ============================================================================
-- PHASE 2: Playlist Management
-- ============================================================================

-- Table for user's YouTube playlists
CREATE TABLE IF NOT EXISTS youtube_playlists (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id INTEGER NOT NULL REFERENCES connected_youtube_channels(id) ON DELETE CASCADE,
    youtube_playlist_id VARCHAR(255) UNIQUE NOT NULL,
    title VARCHAR(255) NOT NULL,
    description TEXT,
    privacy_status VARCHAR(20) DEFAULT 'private' CHECK (privacy_status IN ('public', 'private', 'unlisted')),
    thumbnail_url TEXT,
    video_count INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(user_id, youtube_playlist_id)
);

-- Junction table for playlist items
CREATE TABLE IF NOT EXISTS youtube_playlist_items (
    id SERIAL PRIMARY KEY,
    playlist_id INTEGER NOT NULL REFERENCES youtube_playlists(id) ON DELETE CASCADE,
    youtube_video_id VARCHAR(255) NOT NULL,
    youtube_playlist_item_id VARCHAR(255) UNIQUE,
    position INTEGER NOT NULL,
    video_title VARCHAR(255),
    video_thumbnail_url TEXT,
    added_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(playlist_id, youtube_video_id)
);

-- ============================================================================
-- PHASE 3: Analytics & Insights
-- ============================================================================

-- Video-level analytics cache (from YouTube Analytics API)
CREATE TABLE IF NOT EXISTS youtube_video_analytics (
    id SERIAL PRIMARY KEY,
    youtube_video_id VARCHAR(255) NOT NULL,
    metric_date DATE NOT NULL,
    views BIGINT DEFAULT 0,
    watch_time_minutes BIGINT DEFAULT 0,
    average_view_duration INTEGER,
    average_view_percentage DECIMAL(5,2),
    likes INTEGER DEFAULT 0,
    dislikes INTEGER DEFAULT 0,
    comments INTEGER DEFAULT 0,
    shares INTEGER DEFAULT 0,
    subscribers_gained INTEGER DEFAULT 0,
    subscribers_lost INTEGER DEFAULT 0,
    estimated_revenue DECIMAL(10, 2),
    fetched_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(youtube_video_id, metric_date)
);

-- Channel-level analytics cache
CREATE TABLE IF NOT EXISTS youtube_channel_analytics (
    id SERIAL PRIMARY KEY,
    channel_id INTEGER NOT NULL REFERENCES connected_youtube_channels(id) ON DELETE CASCADE,
    metric_date DATE NOT NULL,
    views BIGINT DEFAULT 0,
    watch_time_minutes BIGINT DEFAULT 0,
    subscribers_gained INTEGER DEFAULT 0,
    subscribers_lost INTEGER DEFAULT 0,
    estimated_revenue DECIMAL(10, 2),
    demographics JSONB,  -- {age_groups: {...}, gender: {...}, geography: {...}}
    traffic_sources JSONB,  -- {youtube_search: N, external: N, suggested_videos: N}
    device_types JSONB,  -- {mobile: N, desktop: N, tv: N, tablet: N}
    fetched_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(channel_id, metric_date)
);

-- ============================================================================
-- PHASE 4: Comment Moderation
-- ============================================================================

-- YouTube comments cache for moderation
CREATE TABLE IF NOT EXISTS youtube_comments (
    id SERIAL PRIMARY KEY,
    youtube_comment_id VARCHAR(255) UNIQUE NOT NULL,
    youtube_video_id VARCHAR(255) NOT NULL,
    parent_comment_id VARCHAR(255),  -- NULL for top-level comments
    author_name VARCHAR(255),
    author_channel_id VARCHAR(255),
    author_profile_image_url TEXT,
    text_display TEXT,
    text_original TEXT,
    like_count INTEGER DEFAULT 0,
    can_reply BOOLEAN DEFAULT true,
    moderation_status VARCHAR(50),  -- published, heldForReview, likelySpam, rejected
    published_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ,
    fetched_at TIMESTAMPTZ DEFAULT NOW()
);

-- ============================================================================
-- PHASE 5: Captions/Subtitles
-- ============================================================================

-- Caption tracks metadata
CREATE TABLE IF NOT EXISTS youtube_captions (
    id SERIAL PRIMARY KEY,
    youtube_video_id VARCHAR(255) NOT NULL,
    youtube_caption_id VARCHAR(255) UNIQUE NOT NULL,
    language VARCHAR(10) NOT NULL,  -- ISO 639-1 code (en, es, fr, etc.)
    name VARCHAR(255),  -- Display name for the caption track
    track_kind VARCHAR(50),  -- standard, ASR, forced
    is_auto_generated BOOLEAN DEFAULT false,
    is_cc BOOLEAN DEFAULT false,  -- Closed captions
    is_draft BOOLEAN DEFAULT false,
    local_file_path TEXT,  -- If we store caption files locally
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- ============================================================================
-- INDEXES FOR PERFORMANCE
-- ============================================================================

-- Playlist indexes
CREATE INDEX IF NOT EXISTS idx_youtube_playlists_user_id ON youtube_playlists(user_id);
CREATE INDEX IF NOT EXISTS idx_youtube_playlists_channel_id ON youtube_playlists(channel_id);
CREATE INDEX IF NOT EXISTS idx_youtube_playlist_items_playlist_id ON youtube_playlist_items(playlist_id);
CREATE INDEX IF NOT EXISTS idx_youtube_playlist_items_video_id ON youtube_playlist_items(youtube_video_id);

-- Analytics indexes (critical for time-series queries)
CREATE INDEX IF NOT EXISTS idx_youtube_video_analytics_video_id ON youtube_video_analytics(youtube_video_id);
CREATE INDEX IF NOT EXISTS idx_youtube_video_analytics_date ON youtube_video_analytics(metric_date);
CREATE INDEX IF NOT EXISTS idx_youtube_video_analytics_video_date ON youtube_video_analytics(youtube_video_id, metric_date);
CREATE INDEX IF NOT EXISTS idx_youtube_channel_analytics_channel_id ON youtube_channel_analytics(channel_id);
CREATE INDEX IF NOT EXISTS idx_youtube_channel_analytics_date ON youtube_channel_analytics(metric_date);

-- Upload status tracking indexes
CREATE INDEX IF NOT EXISTS idx_youtube_uploads_deleted_at ON youtube_uploads(deleted_at) WHERE deleted_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_youtube_uploads_scheduled ON youtube_uploads(scheduled_publish_at) WHERE is_scheduled = true;

-- Comment indexes
CREATE INDEX IF NOT EXISTS idx_youtube_comments_video_id ON youtube_comments(youtube_video_id);
CREATE INDEX IF NOT EXISTS idx_youtube_comments_parent_id ON youtube_comments(parent_comment_id);

-- Caption indexes
CREATE INDEX IF NOT EXISTS idx_youtube_captions_video_id ON youtube_captions(youtube_video_id);

-- ============================================================================
-- UPDATED_AT TRIGGERS
-- ============================================================================

-- Trigger function for updating updated_at timestamp
CREATE OR REPLACE FUNCTION update_youtube_playlists_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Apply trigger to playlists table
CREATE TRIGGER trigger_youtube_playlists_updated_at
    BEFORE UPDATE ON youtube_playlists
    FOR EACH ROW
    EXECUTE FUNCTION update_youtube_playlists_updated_at();

-- ============================================================================
-- COMMENTS
-- ============================================================================

COMMENT ON TABLE youtube_playlists IS 'User playlists from connected YouTube channels';
COMMENT ON TABLE youtube_playlist_items IS 'Videos within playlists with ordering';
COMMENT ON TABLE youtube_video_analytics IS 'Cached video analytics from YouTube Analytics API (24hr TTL)';
COMMENT ON TABLE youtube_channel_analytics IS 'Cached channel analytics with demographics and traffic sources';
COMMENT ON TABLE youtube_comments IS 'YouTube comments cache for moderation features (1hr TTL)';
COMMENT ON TABLE youtube_captions IS 'Caption/subtitle tracks for uploaded videos';

COMMENT ON COLUMN youtube_uploads.deleted_at IS 'Soft delete timestamp - NULL means active';
COMMENT ON COLUMN youtube_uploads.metadata_updated_at IS 'Last time video metadata was edited on YouTube';
COMMENT ON COLUMN youtube_uploads.custom_thumbnail_path IS 'Path to custom thumbnail if uploaded';
COMMENT ON COLUMN youtube_uploads.is_resumable IS 'TRUE if video was uploaded using resumable upload (large files)';
COMMENT ON COLUMN connected_youtube_channels.requires_reauth IS 'TRUE if channel needs re-authentication for new OAuth scopes';
