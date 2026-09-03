-- output_videos.duration_seconds was NUMERIC; Rust expects Option<f64>
-- (FLOAT8). Every persist of a tool-output video failed with
-- "mismatched types: Rust type Option<f64> (as SQL type FLOAT8) is not
-- compatible with SQL type NUMERIC" — chat output persistence + media
-- review artifact registration silently lost for every clip job.
-- Fixed live Sep 3 2026; this migration makes it reproducible.

ALTER TABLE output_videos
    ALTER COLUMN duration_seconds SET DATA TYPE double precision
    USING duration_seconds::double precision;
