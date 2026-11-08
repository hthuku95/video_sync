-- Campaigns app-wide ON/OFF switch (owner directive Nov 8 2026, GLM session).
--
-- `campaigns` is a SYSTEM GROUP (not a managed service): it gates monthly-campaign
-- subscriptions across the product —
--   * `POST /api/campaigns` creation (503 "coming soon" when OFF),
--   * `GET /api/features` (public; the five TS apps render Coming-Soon cards),
--   * landing_page/education prospect outreach copy (bundles-only when OFF).
-- One-time bundles ($50-$100 10-30 videos) and the studio DFY storefront
-- (videosync_studio) are NOT affected — studio never advertises campaigns.
--
-- Seeds OFF because renders are currently paused (Zernio billing). Flip from
-- the terminal (`POST /api/admin/campaigns-switch`) or the admin dashboard
-- button; the row flips at runtime with no redeploy.

INSERT INTO service_flags (service, enabled, updated_by, updated_at)
VALUES ('campaigns', FALSE, 'seed-campaign-switch-nov8', NOW())
ON CONFLICT (service) DO NOTHING;
