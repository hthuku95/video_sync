-- Migration: Phase resumption columns for clipping_jobs
--
-- viral_moments_json: Stores the full VideoAnalysis (including viral moments) from Phase A
--   as JSONB. Populated immediately after Gemini analysis completes. Allows retry jobs
--   to skip Phase A (Gemini re-analysis) and resume directly from Phase B/C/E.
--
-- analysis_quality: Overall quality score from Phase A (0.0–1.0). Stored for quick
--   filtering without deserializing the full JSONB blob.
--
-- resume_from: When auto_retry_failed_jobs resets a failed job to 'pending', it sets
--   this column to the appropriate resume point ('analyzed', 'downloaded', or
--   'clips_extracted'). execute_clipping_job reads this column to skip completed phases.
--   Cleared at the start of each execution to avoid stale hints on subsequent failures.

ALTER TABLE clipping_jobs
    ADD COLUMN IF NOT EXISTS viral_moments_json JSONB,
    ADD COLUMN IF NOT EXISTS analysis_quality FLOAT,
    ADD COLUMN IF NOT EXISTS resume_from VARCHAR(50);
