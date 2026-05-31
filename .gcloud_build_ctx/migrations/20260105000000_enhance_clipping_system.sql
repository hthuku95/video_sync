-- Enhanced YouTube Clipping System Migration
-- Adds backward scanning, session memory, cooldown tracking, and daily clip limits

-- Table 1: Track ALL clipped source videos (enables backward scanning)
CREATE TABLE clipped_source_videos (
    id SERIAL PRIMARY KEY,
    source_channel_id INTEGER NOT NULL REFERENCES youtube_source_channels(id) ON DELETE CASCADE,
    video_id VARCHAR(255) NOT NULL,
    video_title TEXT,
    video_published_at TIMESTAMPTZ,
    first_clipped_at TIMESTAMPTZ DEFAULT NOW(),
    created_at TIMESTAMPTZ DEFAULT NOW(),

    -- Ensure each video is tracked once per source channel
    UNIQUE(source_channel_id, video_id)
);

CREATE INDEX idx_clipped_videos_channel ON clipped_source_videos(source_channel_id);
CREATE INDEX idx_clipped_videos_video_id ON clipped_source_videos(video_id);
CREATE INDEX idx_clipped_videos_first_clipped ON clipped_source_videos(first_clipped_at);

-- Table 2: Session memory - store videos found but blocked by cooldown/limits
CREATE TABLE pending_unclipped_videos (
    id SERIAL PRIMARY KEY,
    linkage_id INTEGER NOT NULL REFERENCES youtube_channel_linkages(id) ON DELETE CASCADE,
    video_id VARCHAR(255) NOT NULL,
    video_title TEXT,
    video_published_at TIMESTAMPTZ,
    discovered_at TIMESTAMPTZ DEFAULT NOW(),
    created_at TIMESTAMPTZ DEFAULT NOW(),

    -- Prevent duplicates per linkage
    UNIQUE(linkage_id, video_id)
);

CREATE INDEX idx_pending_videos_linkage ON pending_unclipped_videos(linkage_id);
CREATE INDEX idx_pending_videos_discovered ON pending_unclipped_videos(discovered_at);

-- Modification 1: Add session tracking to channel linkages
ALTER TABLE youtube_channel_linkages
ADD COLUMN last_clipping_session_at TIMESTAMPTZ,
ADD COLUMN clipping_cooldown_hours INTEGER DEFAULT 24 CHECK (clipping_cooldown_hours > 0);

CREATE INDEX idx_linkages_last_session ON youtube_channel_linkages(last_clipping_session_at);

-- Modification 2: Add destination channel tracking to extracted clips (for daily counting)
ALTER TABLE extracted_clips
ADD COLUMN destination_channel_id INTEGER REFERENCES connected_youtube_channels(id) ON DELETE SET NULL;

CREATE INDEX idx_extracted_clips_published_date ON extracted_clips(published_at)
WHERE upload_status = 'published';

CREATE INDEX idx_extracted_clips_destination ON extracted_clips(destination_channel_id);

-- Backfill destination_channel_id for existing clips
UPDATE extracted_clips ec
SET destination_channel_id = (
    SELECT ycl.destination_channel_id
    FROM clipping_jobs cj
    JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
    WHERE cj.id = ec.clipping_job_id
);
