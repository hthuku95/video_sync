-- Record which object-storage backend holds each media object.
--
-- Context: R2 is the primary backend and every object created so far lives on
-- R2. When R2 is unavailable (e.g. account billing block / NotEntitled) the app
-- now fails over to an AWS S3 bucket (account 960882269041, us-east-1). We must
-- remember, per object, which backend stored it so reads can be served from the
-- right place.
--
-- `generated_artifacts` already carries a `storage_backend` column; this adds it
-- (defaulting to 'r2' for every pre-existing row) to the remaining media tables.
--
-- Values: 'r2' (Cloudflare R2) | 's3' (AWS S3 fallback). Safe to re-run.

ALTER TABLE deliveries               ADD COLUMN IF NOT EXISTS storage_backend TEXT NOT NULL DEFAULT 'r2';
ALTER TABLE extracted_clips          ADD COLUMN IF NOT EXISTS storage_backend TEXT NOT NULL DEFAULT 'r2';
ALTER TABLE clipping_jobs            ADD COLUMN IF NOT EXISTS storage_backend TEXT NOT NULL DEFAULT 'r2';
ALTER TABLE manual_clipping_clips    ADD COLUMN IF NOT EXISTS storage_backend TEXT NOT NULL DEFAULT 'r2';
ALTER TABLE campaign_files           ADD COLUMN IF NOT EXISTS storage_backend TEXT NOT NULL DEFAULT 'r2';
ALTER TABLE campaign_posts           ADD COLUMN IF NOT EXISTS storage_backend TEXT NOT NULL DEFAULT 'r2';
ALTER TABLE output_videos            ADD COLUMN IF NOT EXISTS storage_backend TEXT NOT NULL DEFAULT 'r2';
ALTER TABLE gig_sample_videos        ADD COLUMN IF NOT EXISTS storage_backend TEXT NOT NULL DEFAULT 'r2';
ALTER TABLE service_portfolio_samples ADD COLUMN IF NOT EXISTS storage_backend TEXT NOT NULL DEFAULT 'r2';
ALTER TABLE test_results             ADD COLUMN IF NOT EXISTS storage_backend TEXT NOT NULL DEFAULT 'r2';
