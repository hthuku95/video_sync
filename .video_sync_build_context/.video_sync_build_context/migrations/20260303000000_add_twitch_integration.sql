-- Twitch TV integration: source channels, YouTube→Twitch mapping, VOD tracking, token cache

-- Twitch broadcaster accounts that can serve as fallback video sources
CREATE TABLE twitch_source_channels (
    id SERIAL PRIMARY KEY,
    broadcaster_id VARCHAR(255) NOT NULL UNIQUE,
    broadcaster_login VARCHAR(255) NOT NULL,
    display_name VARCHAR(255) NOT NULL,
    profile_image_url TEXT,
    is_active BOOLEAN DEFAULT true,
    last_polled_at TIMESTAMPTZ,
    last_video_checked VARCHAR(255),
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- 1:1 mapping: each YouTube source channel may have one Twitch equivalent
CREATE TABLE youtube_twitch_channel_mappings (
    id SERIAL PRIMARY KEY,
    youtube_source_channel_id INTEGER NOT NULL UNIQUE
        REFERENCES youtube_source_channels(id) ON DELETE CASCADE,
    twitch_source_channel_id INTEGER NOT NULL
        REFERENCES twitch_source_channels(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Tracks which Twitch VODs have been used as clip sources (prevents re-use)
CREATE TABLE clipped_twitch_videos (
    id SERIAL PRIMARY KEY,
    twitch_channel_id INTEGER NOT NULL
        REFERENCES twitch_source_channels(id) ON DELETE CASCADE,
    video_id VARCHAR(255) NOT NULL,
    video_title TEXT,
    clipping_job_id INTEGER REFERENCES clipping_jobs(id) ON DELETE SET NULL,
    clipped_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(twitch_channel_id, video_id)
);

-- Single-row cache for Twitch app access token (~58-day lifetime)
CREATE TABLE twitch_app_token (
    id INTEGER PRIMARY KEY DEFAULT 1,
    access_token TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Track Gemini AI mapping status per YouTube source channel
ALTER TABLE youtube_source_channels
    ADD COLUMN twitch_mapping_status VARCHAR(30) DEFAULT 'unmapped'
        CHECK (twitch_mapping_status IN ('unmapped', 'mapped', 'no_twitch_equivalent'));

-- Extend clipping_jobs for Twitch fallback tracking
ALTER TABLE clipping_jobs
    ADD COLUMN used_twitch_fallback BOOLEAN DEFAULT false,
    ADD COLUMN twitch_video_id VARCHAR(255),
    ADD COLUMN active_video_url TEXT;
