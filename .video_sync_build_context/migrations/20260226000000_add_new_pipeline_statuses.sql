-- Migration: Add new 5-phase pipeline status values to clipping_jobs constraint
--
-- The new architecture introduces three additional statuses:
--   analyzed       - Phase A complete (Gemini found quality moments), before download
--   no_clips_found - Phase A fast-fail (Gemini found no quality viral moments)
--   vectorizing    - Phase D in progress (storing embedding in Qdrant video_content)

ALTER TABLE clipping_jobs
  DROP CONSTRAINT IF EXISTS valid_status_values;

ALTER TABLE clipping_jobs
  ADD CONSTRAINT valid_status_values CHECK (
    status::text = ANY (ARRAY[
      'pending',
      'analyzing',
      'analyzed',
      'no_clips_found',
      'downloading',
      'downloaded',
      'extracting_clips',
      'clips_extracted',
      'vectorizing',
      'vectorized',
      'posting',
      'completed',
      'failed',
      'cancelled'
    ]::text[])
  );
