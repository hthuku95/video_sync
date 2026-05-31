-- Studio payments: tracks all one-time purchases from the studio store
-- (both PayPal and USDC/crypto), regardless of whether the buyer has a
-- user account. This is separate from user_payment_events which tracks
-- subscription billing for authenticated users.
CREATE TABLE IF NOT EXISTS studio_payments (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    offer_id        TEXT NOT NULL,
    offer_name      TEXT NOT NULL,
    amount_cents    INT NOT NULL,
    currency        TEXT NOT NULL DEFAULT 'USD',
    payment_method  TEXT NOT NULL,   -- 'paypal_sandbox' | 'paypal_live' | 'usdc_base'
    status          TEXT NOT NULL DEFAULT 'pending',  -- 'pending' | 'completed' | 'failed' | 'refunded'
    paypal_order_id TEXT,
    paypal_capture_id TEXT,
    tx_hash         TEXT,
    payer_address   TEXT,            -- wallet address for USDC, PayPal payer email
    buyer_email     TEXT,
    buyer_name      TEXT,
    raw_meta        JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_studio_payments_status
    ON studio_payments(status);
CREATE INDEX IF NOT EXISTS idx_studio_payments_created
    ON studio_payments(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_studio_payments_paypal_order
    ON studio_payments(paypal_order_id)
    WHERE paypal_order_id IS NOT NULL;

-- App configuration key-value store.
-- Allows runtime configuration changes (like PayPal sandbox/live toggle)
-- without redeploying. Falls back to env vars when no row exists.
CREATE TABLE IF NOT EXISTS app_config (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by  INT REFERENCES users(id)
);

-- Seed default config from env var if available (runs every migration,
-- idempotent via ON CONFLICT).
INSERT INTO app_config (key, value, updated_at)
VALUES ('paypal_env', 'sandbox', NOW())
ON CONFLICT (key) DO NOTHING;
