-- YouTube Clipping Feature Migration
-- Adds tables for monitoring external channels, creating channel linkages,
-- tracking clipping jobs, storing extracted clips, and managing polling schedule

-- Source channels to monitor (external channels like Mr Beast)
CREATE TABLE youtube_source_channels (
    id SERIAL PRIMARY KEY,
    channel_id VARCHAR(255) NOT NULL UNIQUE,        -- YouTube channel ID
    channel_name VARCHAR(255) NOT NULL,
    channel_thumbnail_url TEXT,
    subscriber_count BIGINT,
    is_active BOOLEAN DEFAULT true,
    polling_interval_minutes INTEGER DEFAULT 30,   -- How often to check for new videos
    last_polled_at TIMESTAMPTZ,
    last_video_checked VARCHAR(255),                -- Latest video ID we've processed
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Channel linkages (one source → many destinations)
CREATE TABLE youtube_channel_linkages (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    source_channel_id INTEGER NOT NULL REFERENCES youtube_source_channels(id) ON DELETE CASCADE,
    destination_channel_id INTEGER NOT NULL REFERENCES connected_youtube_channels(id) ON DELETE CASCADE,
    is_active BOOLEAN DEFAULT true,

    -- Clipping configuration
    clips_per_video INTEGER DEFAULT 2 CHECK (clips_per_video BETWEEN 1 AND 4),
    min_clip_duration_seconds INTEGER DEFAULT 60,
    max_clip_duration_seconds INTEGER DEFAULT 120,

    -- Statistics
    total_clips_generated INTEGER DEFAULT 0,
    total_clips_posted INTEGER DEFAULT 0,
    last_clip_generated_at TIMESTAMPTZ,

    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),

    UNIQUE(source_channel_id, destination_channel_id)  -- Prevent duplicate links
);

-- Clipping jobs (track video processing)
CREATE TABLE clipping_jobs (
    id SERIAL PRIMARY KEY,
    linkage_id INTEGER NOT NULL REFERENCES youtube_channel_linkages(id) ON DELETE CASCADE,
    source_video_id VARCHAR(255) NOT NULL,           -- YouTube video ID from source channel
    source_video_title TEXT,
    source_video_duration_seconds INTEGER,
    local_video_path TEXT,                           -- Path after yt-dlp download

    status VARCHAR(50) DEFAULT 'pending',            -- pending, downloading, downloaded, analyzing,
                                                      -- extracting_clips, reviewing, posting, completed, failed
    current_step VARCHAR(100),
    progress_percent INTEGER DEFAULT 0,
    error_message TEXT,

    -- Timestamps
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Extracted clips
CREATE TABLE extracted_clips (
    id SERIAL PRIMARY KEY,
    clipping_job_id INTEGER NOT NULL REFERENCES clipping_jobs(id) ON DELETE CASCADE,
    clip_number INTEGER NOT NULL,                    -- 1, 2, 3, or 4
    local_clip_path TEXT NOT NULL,

    -- AI-identified segment
    start_time_seconds FLOAT NOT NULL,
    end_time_seconds FLOAT NOT NULL,
    duration_seconds FLOAT NOT NULL,

    -- AI-generated metadata
    ai_title VARCHAR(255),
    ai_description TEXT,
    ai_tags TEXT[],                                  -- Array of suggested tags
    ai_confidence_score FLOAT,                        -- 0-1 confidence in clip quality
    viral_factors TEXT[],                            -- Reasons: ["hook detected", "dramatic reveal", etc.]

    -- YouTube upload tracking
    youtube_video_id VARCHAR(255),
    youtube_url TEXT,
    upload_status VARCHAR(50) DEFAULT 'pending',      -- pending, uploading, published, failed
    published_at TIMESTAMPTZ,
    upload_error TEXT,

    -- Statistics (populated after posting)
    views_24h INTEGER DEFAULT 0,
    likes_24h INTEGER DEFAULT 0,
    comments_24h INTEGER DEFAULT 0,

    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Polling schedule tracking
CREATE TABLE clipping_poll_schedule (
    id SERIAL PRIMARY KEY,
    source_channel_id INTEGER NOT NULL REFERENCES youtube_source_channels(id) ON DELETE CASCADE UNIQUE,
    next_poll_at TIMESTAMPTZ NOT NULL,
    is_polling BOOLEAN DEFAULT false,                 -- Prevent concurrent polls
    last_poll_duration_ms INTEGER,
    consecutive_failures INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Indexes for performance
CREATE INDEX idx_source_channels_active ON youtube_source_channels(is_active);
CREATE INDEX idx_source_channels_last_polled ON youtube_source_channels(last_polled_at);
CREATE INDEX idx_channel_linkages_user ON youtube_channel_linkages(user_id);
CREATE INDEX idx_channel_linkages_source ON youtube_channel_linkages(source_channel_id);
CREATE INDEX idx_channel_linkages_destination ON youtube_channel_linkages(destination_channel_id);
CREATE INDEX idx_channel_linkages_active ON youtube_channel_linkages(is_active);
CREATE INDEX idx_clipping_jobs_status ON clipping_jobs(status);
CREATE INDEX idx_clipping_jobs_video ON clipping_jobs(source_video_id);
CREATE INDEX idx_extracted_clips_job ON extracted_clips(clipping_job_id);
CREATE INDEX idx_extracted_clips_status ON extracted_clips(upload_status);
CREATE INDEX idx_poll_schedule_next_poll ON clipping_poll_schedule(next_poll_at);

-- Triggers for updated_at
CREATE TRIGGER update_source_channels_updated_at BEFORE UPDATE ON youtube_source_channels
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
CREATE TRIGGER update_channel_linkages_updated_at BEFORE UPDATE ON youtube_channel_linkages
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
CREATE TRIGGER update_clipping_jobs_updated_at BEFORE UPDATE ON clipping_jobs
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
CREATE TRIGGER update_extracted_clips_updated_at BEFORE UPDATE ON extracted_clips
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
CREATE TRIGGER update_poll_schedule_updated_at BEFORE UPDATE ON clipping_poll_schedule
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
