-- Education video credits bundles (Oct 8, 2026)
-- Gives the `education` service its second selling type: alongside the
-- monthly campaign subscription it can now be sold a-la-carte as a
-- $50/10 or $100/30 bundle (mirrors website_video_bundles for landing_page).
-- One row per bundle purchase; credits are consumed at generation time.

CREATE TABLE IF NOT EXISTS education_video_bundles (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id           INTEGER NOT NULL REFERENCES users(id),
    offer_id          TEXT NOT NULL,            -- 'education-video-10' | 'education-video-30'
    credits_purchased INTEGER NOT NULL,
    credits_used      INTEGER NOT NULL DEFAULT 0,
    topic             TEXT,                      -- lesson topic the bundle is for
    source_url        TEXT,                      -- optional reference/course URL
    payment_status    TEXT NOT NULL DEFAULT 'pending',  -- 'pending' | 'completed' | 'failed' | 'refunded'
    payment_method    TEXT,                     -- 'paypal_sandbox' | 'paypal_live' | 'usdc_base'
    paypal_order_id   TEXT,
    paypal_capture_id TEXT,
    tx_hash           TEXT,
    amount_cents      INTEGER NOT NULL DEFAULT 0,
    currency          TEXT NOT NULL DEFAULT 'USD',
    raw_meta          JSONB,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    paid_at           TIMESTAMPTZ,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_education_video_bundles_user
    ON education_video_bundles(user_id);
CREATE INDEX IF NOT EXISTS idx_education_video_bundles_paypal_order
    ON education_video_bundles(paypal_order_id)
    WHERE paypal_order_id IS NOT NULL;

-- The `education` service is now sold directly, so it must be renderable.
-- Landing-page style seed lives in 20260928000000_service_flags.sql.
INSERT INTO service_flags (service, enabled, updated_at)
VALUES ('education', TRUE, NOW())
ON CONFLICT (service) DO UPDATE SET enabled = TRUE, updated_at = NOW();
