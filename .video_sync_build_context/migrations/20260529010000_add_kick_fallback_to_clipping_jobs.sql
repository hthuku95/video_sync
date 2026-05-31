-- Kick.com fallback in automatic clipping download chain
-- Adds fields to track when a clipping job falls back to downloading from Kick.com

ALTER TABLE clipping_jobs
    ADD COLUMN IF NOT EXISTS used_kick_fallback BOOLEAN DEFAULT false,
    ADD COLUMN IF NOT EXISTS kick_channel_slug VARCHAR(255),
    ADD COLUMN IF NOT EXISTS kick_video_url TEXT;
