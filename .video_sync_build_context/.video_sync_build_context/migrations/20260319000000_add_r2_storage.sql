-- R2 Object Storage Integration (Mar 19, 2026)
-- Adds R2 key + presigned URL columns to extracted_clips and clipping_jobs.
-- local_clip_path kept for backward compatibility; r2_key is the canonical
-- storage reference once R2 upload completes.

-- ── extracted_clips ──────────────────────────────────────────────────────────

ALTER TABLE extracted_clips
    ADD COLUMN IF NOT EXISTS r2_clip_key        TEXT,            -- clips/{job_id}/clip_{n}.mp4
    ADD COLUMN IF NOT EXISTS r2_thumb_key       TEXT,            -- clips/{job_id}/thumb_{n}.jpg
    ADD COLUMN IF NOT EXISTS r2_clip_url        TEXT,            -- presigned GET URL (24 h)
    ADD COLUMN IF NOT EXISTS r2_clip_url_expires_at TIMESTAMPTZ; -- when the presigned URL expires

-- ── clipping_jobs ─────────────────────────────────────────────────────────────

ALTER TABLE clipping_jobs
    ADD COLUMN IF NOT EXISTS r2_raw_key         TEXT,            -- raw/{user_id}/{video_id}.mp4
    ADD COLUMN IF NOT EXISTS r2_upload_status   TEXT DEFAULT 'pending';
                                                                  -- pending | uploading | done | failed

-- ── indexes ───────────────────────────────────────────────────────────────────

CREATE INDEX IF NOT EXISTS idx_extracted_clips_r2_key  ON extracted_clips(r2_clip_key);
CREATE INDEX IF NOT EXISTS idx_clipping_jobs_r2_status ON clipping_jobs(r2_upload_status);
