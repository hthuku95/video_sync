-- Instagram prospect fields + instagram_leads table

ALTER TABLE prospects
    ADD COLUMN IF NOT EXISTS instagram_username TEXT;

-- Stores individual Instagram leads from PhantomBuster scrapes
CREATE TABLE IF NOT EXISTS instagram_leads (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username         TEXT NOT NULL,
    full_name        TEXT,
    bio              TEXT,
    followers_count  BIGINT,
    following_count  BIGINT,
    posts_count      INT,
    profile_url      TEXT,
    profile_pic_url  TEXT,
    is_private       BOOLEAN DEFAULT FALSE,
    is_verified      BOOLEAN DEFAULT FALSE,
    category         TEXT,           -- niche/hashtag category used to find them
    hashtag_source   TEXT,           -- the hashtag that surfaced them
    email            TEXT,
    external_url     TEXT,           -- link in bio
    dm_script        TEXT,           -- AI-generated cold DM
    contact_status   TEXT NOT NULL DEFAULT 'new',   -- new | contacted | replied | converted | skipped
    pb_job_id        UUID REFERENCES phantombuster_jobs(id) ON DELETE SET NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(username)
);

CREATE INDEX IF NOT EXISTS instagram_leads_category_idx      ON instagram_leads(category);
CREATE INDEX IF NOT EXISTS instagram_leads_contact_status_idx ON instagram_leads(contact_status);
CREATE INDEX IF NOT EXISTS instagram_leads_followers_idx      ON instagram_leads(followers_count DESC);
