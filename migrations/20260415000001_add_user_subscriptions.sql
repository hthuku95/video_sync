-- Regular-user paywall: 7-day free trial → $15/mo USDC on heavy-compute
-- features. Clipping / manual clipping stay whitelisted (team-only),
-- unaffected by these columns.
--
-- Every user that exists RIGHT NOW is grandfathered (permanent free tier).
-- New signups land in 'trial' status with trial_ends_at = signup + 7 days.
-- When trial expires the subscription_middleware flips them to 'expired'
-- and returns HTTP 402 on paywalled routes until they upgrade to 'active'.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS trial_ends_at             TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS subscription_status       TEXT,
    ADD COLUMN IF NOT EXISTS subscription_tier         TEXT,
    ADD COLUMN IF NOT EXISTS subscription_active_until TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS last_payment_receipt_id   TEXT,
    ADD COLUMN IF NOT EXISTS last_payment_at           TIMESTAMPTZ;

-- Backfill: everyone who exists RIGHT NOW is grandfathered (stays free
-- forever). Only rows with NULL status are touched — idempotent.
UPDATE users
SET subscription_status = 'grandfathered'
WHERE subscription_status IS NULL;

-- From this point forward, the default for new rows is 'trial' (set by
-- the register handler, not a column default, because we also want to
-- set trial_ends_at in the same INSERT).
CREATE INDEX IF NOT EXISTS idx_users_subscription_status
    ON users(subscription_status);
CREATE INDEX IF NOT EXISTS idx_users_trial_ends_at
    ON users(trial_ends_at)
    WHERE subscription_status = 'trial';

-- Append-only audit log of trial/payment/expiry events per user.
-- Lets admin see the billing timeline without modifying the user row.
CREATE TABLE IF NOT EXISTS user_payment_events (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      INT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    event_type   TEXT NOT NULL,     -- 'trial_started' | 'paid' | 'expired' | 'refunded' | 'cancelled'
    amount_usdc  NUMERIC(10, 2),    -- NULL for non-financial events
    tx_hash      TEXT,              -- Base chain tx hash for 'paid'
    event_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    raw_meta     JSONB               -- freeform: IP, user-agent, x402 payload hash, etc.
);

CREATE INDEX IF NOT EXISTS idx_user_payment_events_user_time
    ON user_payment_events(user_id, event_at DESC);
