-- Allow generated-summary fallback jobs to remain active while their
-- delivery render is still running.
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
            'fallback_rendering',
            'completed',
            'failed',
            'cancelled',
            'discarded'
        ]::text[])
    );
