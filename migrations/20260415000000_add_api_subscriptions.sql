-- API subscriptions for white-label / agency licensing.
-- Agencies pay $99-$299 USDC on Base via x402, get an API key tied to a
-- monthly quota. Reuses the same x402 module + facilitator settlement we
-- shipped for /delivery/:id.

CREATE TABLE IF NOT EXISTS api_subscriptions (
    id                       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Buyer identity. Email is optional (anon allowed for crypto-native
    -- buyers) but useful for support + recovery if they lose the key.
    email                    TEXT,
    contact_handle           TEXT,                -- Telegram / X / Discord handle
    -- The actual API key the buyer uses in `Authorization: Bearer <key>`.
    -- Generated server-side post-payment. Stored hashed would be safer but
    -- we keep plain for now since this is a starter implementation.
    api_key                  TEXT UNIQUE NOT NULL,
    -- 'starter' (1k clips, 500 thumbs, $99) | 'pro' (5k clips, 2.5k thumbs, $199)
    tier                     TEXT NOT NULL DEFAULT 'starter',
    monthly_clip_quota       INT  NOT NULL DEFAULT 1000,
    monthly_thumbnail_quota  INT  NOT NULL DEFAULT 500,
    monthly_animation_quota  INT  NOT NULL DEFAULT 50,
    -- Usage counters, reset by a background job at start of each cycle.
    clips_used_this_period      INT NOT NULL DEFAULT 0,
    thumbnails_used_this_period INT NOT NULL DEFAULT 0,
    animations_used_this_period INT NOT NULL DEFAULT 0,
    -- When the current paid period ends. 30 days from payment.
    active_until             TIMESTAMPTZ,
    -- x402 receipt — the on-chain tx hash returned by the facilitator.
    payment_receipt_id       TEXT,
    payment_amount_usdc      NUMERIC(10, 2),
    status                   TEXT NOT NULL DEFAULT 'pending', -- pending | active | expired | cancelled
    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_api_subscriptions_api_key
    ON api_subscriptions(api_key);
CREATE INDEX IF NOT EXISTS idx_api_subscriptions_active
    ON api_subscriptions(status, active_until);
CREATE INDEX IF NOT EXISTS idx_api_subscriptions_email
    ON api_subscriptions(email);
