-- Agency tiers (owner directive Oct 2026): per-user per-app subscription state
-- plus account caps. Base tiers included free with the app subscription;
-- agency tiers raise the cap. One row per (user, app); missing row = base.
CREATE TABLE IF NOT EXISTS user_app_subscriptions (
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    app_slug    TEXT NOT NULL, -- clips|shorts|app|learn|motion
    tier        TEXT NOT NULL DEFAULT 'base', -- base|agency50|agency150
    status      TEXT NOT NULL DEFAULT 'active', -- active|past_due|cancelled
    paid_until  TIMESTAMPTZ,
    payment_ref TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, app_slug)
);
CREATE INDEX IF NOT EXISTS idx_user_app_subs_user ON user_app_subscriptions(user_id);
