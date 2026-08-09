-- Website-URL→Video credits bundles (Aug 6, 2026)
-- Monetizes the landing_page service as an a-la-carte $50/10 or $100/30 bundle.
-- One row per bundle purchase; credits are consumed at generation time.

CREATE TABLE IF NOT EXISTS website_video_bundles (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id           INTEGER NOT NULL REFERENCES users(id),
    offer_id          TEXT NOT NULL,            -- 'website-video-10' | 'website-video-30'
    credits_purchased INTEGER NOT NULL,
    credits_used      INTEGER NOT NULL DEFAULT 0,
    source_url        TEXT,                     -- website URL the bundle is for
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

CREATE INDEX IF NOT EXISTS idx_website_video_bundles_user
    ON website_video_bundles(user_id);
CREATE INDEX IF NOT EXISTS idx_website_video_bundles_paypal_order
    ON website_video_bundles(paypal_order_id)
    WHERE paypal_order_id IS NOT NULL;

-- Associate deliveries with the user who generated them so /api/website-video/videos
-- can list a per-user feed (the deliveries table previously had no user owner).
ALTER TABLE deliveries
    ADD COLUMN IF NOT EXISTS user_id INTEGER REFERENCES users(id);

CREATE INDEX IF NOT EXISTS idx_deliveries_user_id
    ON deliveries(user_id)
    WHERE user_id IS NOT NULL;
