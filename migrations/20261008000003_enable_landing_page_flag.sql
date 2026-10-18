-- Enable the `landing_page` service flag.
--
-- landing_page was seeded FALSE (20260928000000_service_flags.sql) and never
-- flipped. That gate blocks BOTH selling types for App 3 (app.videosync.ink):
--   * one-time URL→video bundles (website_video.rs → AgenticServicePipeline)
--   * monthly campaign subscriptions (campaigns.rs create → service_enabled)
-- App 3 is live and sells bundles, so the flag must be ON for either flow to
-- render. Idempotent: safe to re-run and a no-op if already enabled.
--
-- Companion: education was flipped TRUE in 20261008000002_education_video_bundles.sql.
UPDATE service_flags
   SET enabled = TRUE,
       updated_by = 'enable_two_selling_types_20261008'
 WHERE service = 'landing_page';

-- Seed the row if it is somehow missing (fresh/partial environments).
INSERT INTO service_flags (service, enabled, updated_by)
VALUES ('landing_page', TRUE, 'enable_two_selling_types_20261008')
ON CONFLICT (service) DO UPDATE
   SET enabled = EXCLUDED.enabled,
       updated_by = EXCLUDED.updated_by;
