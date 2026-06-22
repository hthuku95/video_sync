CREATE TABLE IF NOT EXISTS campaign_posts (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    campaign_id         UUID NOT NULL REFERENCES campaigns(id) ON DELETE CASCADE,
    day_number          INTEGER NOT NULL,
    slot_index          INTEGER NOT NULL,
    scheduled_at        TIMESTAMPTZ NOT NULL,
    variation_prompt    TEXT,
    caption             TEXT,
    media_r2_url        TEXT,
    delivery_id         UUID,
    status              TEXT NOT NULL DEFAULT 'pending_generation'
                        CHECK (status IN ('pending_generation', 'rendering', 'scheduled', 'published', 'failed')),
    zernio_post_id      TEXT,
    error_message       TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at        TIMESTAMPTZ,
    UNIQUE (campaign_id, day_number, slot_index)
);

CREATE INDEX IF NOT EXISTS idx_campaign_posts_campaign ON campaign_posts(campaign_id);
CREATE INDEX IF NOT EXISTS idx_campaign_posts_status ON campaign_posts(campaign_id, status, scheduled_at);
CREATE INDEX IF NOT EXISTS idx_campaign_posts_delivery ON campaign_posts(delivery_id);
