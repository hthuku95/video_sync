-- Telegram opportunity surface for the prospect finder.
-- Phase 1 (this migration): data model + manual-entry form. The user
-- pastes a message they saw in a Telegram channel, AI scores it as a
-- potential paid gig, row lands in telegram_opportunities.
-- Phase 2 (next session): grammers-client MTProto watcher that polls
-- the channels in telegram_watch_channels and auto-inserts.

CREATE TABLE IF NOT EXISTS telegram_watch_channels (
    id          SERIAL PRIMARY KEY,
    channel     TEXT NOT NULL,              -- @handle or numeric id (t.me/foo → "foo")
    keyword_re  TEXT,                        -- optional override regex
    enabled     BOOLEAN DEFAULT TRUE,
    created_at  TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE (channel)
);

CREATE TABLE IF NOT EXISTS telegram_opportunities (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    channel      TEXT NOT NULL,
    message_id   BIGINT,                     -- Telegram internal id (NULL for manual entries)
    sender       TEXT,
    message      TEXT NOT NULL,
    matched_kw   TEXT,                       -- keyword that triggered (phase 2)
    link         TEXT,                       -- t.me/{channel}/{message_id}
    -- AI scoring (same pattern as instagram_leads)
    score        INT,                         -- 0-100
    score_reason TEXT,
    service_type TEXT,                       -- clipping/animations/thumbnails/ugc/full_stack
    -- Workflow state
    status       TEXT NOT NULL DEFAULT 'new', -- new | contacted | won | lost | ignored
    source       TEXT NOT NULL DEFAULT 'manual', -- manual | watcher
    user_id      INT REFERENCES users(id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ DEFAULT NOW(),
    updated_at   TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_telegram_opp_user_status
    ON telegram_opportunities(user_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_telegram_opp_channel
    ON telegram_opportunities(channel);

-- Seed the default watched-channel list the user mentioned. These are
-- public crypto/SaaS job channels where "need editor / need video /
-- paying in USDC" posts land daily. Phase 2 watcher reads this list.
INSERT INTO telegram_watch_channels (channel, enabled) VALUES
    ('cryptojobslist',       TRUE),
    ('cryptojobs',           TRUE),
    ('web3_jobs',            TRUE),
    ('SaaSFounders',         TRUE),
    ('directoryofmarketers', TRUE),
    ('contentcreators_hub',  TRUE)
ON CONFLICT (channel) DO NOTHING;
