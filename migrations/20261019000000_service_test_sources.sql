-- Discovered source URLs for Managed Campaign ("DFY") service quality tests.
--
-- The `dfy_services()` fixtures no longer hardcode streamer/channel handles for
-- the clipping services. Instead the source is discovered at delivery-creation
-- time (Twitch / Kick / YouTube) and cached here, so:
--   * repeated quality-test runs use the SAME source (deterministic), and
--   * re-discovery only happens when the cached URL is stale or the VOD is gone.
--
-- See `portfolio_samples::resolve_service_source_url`.
CREATE TABLE IF NOT EXISTS service_test_sources (
    service_slug     TEXT PRIMARY KEY,
    source_url       TEXT NOT NULL,
    platform         TEXT NOT NULL,
    discovered_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_verified_at TIMESTAMPTZ,
    verify_failures  INTEGER NOT NULL DEFAULT 0
);

COMMENT ON TABLE service_test_sources IS
    'Cached discovered source URLs for Managed Campaign service tests (discover once, persist, reuse).';
