-- Track YouTube publication for generated fallback delivery videos.
-- This lets the worker recover completed fallback summaries without duplicate posts.

ALTER TABLE deliveries
    ADD COLUMN IF NOT EXISTS youtube_video_id TEXT,
    ADD COLUMN IF NOT EXISTS youtube_url TEXT,
    ADD COLUMN IF NOT EXISTS youtube_uploaded_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS youtube_upload_attempted_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS youtube_upload_error TEXT;

CREATE INDEX IF NOT EXISTS idx_deliveries_youtube_upload_recovery
    ON deliveries (status, youtube_upload_attempted_at)
    WHERE youtube_video_id IS NULL;
