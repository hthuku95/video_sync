-- Education posting system (owner directive Nov 13 2026):
-- The legacy YouTube-API system loses clipping and gains education video
-- posting. Linkage semantics change: topic ↔ destination channel (was
-- source channel ↔ destination channel).

-- The NEW linkage: an educational topic connected to a connected YouTube channel.
CREATE TABLE IF NOT EXISTS education_linkages (
    id SERIAL PRIMARY KEY,
    -- The destination channel that receives generated education videos
    connected_channel_id INTEGER NOT NULL REFERENCES connected_youtube_channels(id) ON DELETE CASCADE,
    -- The educational topic for this channel (e.g. 'History', 'Personal Finance',
    -- or a user-typed custom topic). The agent constructs the generation brief
    -- from this topic.
    topic TEXT NOT NULL,
    -- TRUE for the backfilled default (History) linkage
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_by INTEGER REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- One active linkage per channel (topic may be changed by the user)
    UNIQUE (connected_channel_id)
);
CREATE INDEX IF NOT EXISTS idx_education_linkages_channel ON education_linkages(connected_channel_id);

-- Backfill: every existing connected channel gets a default History linkage
-- (owner directive: History is the default educational topic).
INSERT INTO education_linkages (connected_channel_id, topic, is_default, created_by)
SELECT c.id, 'History', TRUE, c.user_id
FROM connected_youtube_channels c
WHERE NOT EXISTS (SELECT 1 FROM education_linkages el WHERE el.connected_channel_id = c.id);

-- One generation + upload job per video.
CREATE TABLE IF NOT EXISTS education_post_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    linkage_id INTEGER NOT NULL REFERENCES education_linkages(id) ON DELETE CASCADE,
    connected_channel_id INTEGER NOT NULL REFERENCES connected_youtube_channels(id) ON DELETE CASCADE,
    -- The topic used for this specific video (defaults to the linkage topic;
    -- may be a subtopic the agent selected within a niche)
    topic TEXT NOT NULL,
    -- The brief the agent constructed from the topic (written to the job
    -- before generation so the run is auditable)
    brief TEXT,
    -- queued | generating | uploading | completed | failed
    status VARCHAR(32) NOT NULL DEFAULT 'queued',
    progress_percent INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    -- Link to the workflow that ran the Education pipeline (auditability)
    workflow_id UUID REFERENCES app_workflows(id),
    -- Output + upload results
    output_url TEXT,
    youtube_video_id VARCHAR(64),
    youtube_url TEXT,
    youtube_title TEXT,
    -- Scheduling
    scheduled_for TIMESTAMPTZ,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_education_post_jobs_status ON education_post_jobs(status);
CREATE INDEX IF NOT EXISTS idx_education_post_jobs_channel ON education_post_jobs(connected_channel_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_education_post_jobs_scheduled ON education_post_jobs(scheduled_for) WHERE status = 'queued';

-- Anti-repeat ledger: topics already posted per channel, so the agent picks
-- fresh subtopics every day.
CREATE TABLE IF NOT EXISTS education_posted_topics (
    id SERIAL PRIMARY KEY,
    connected_channel_id INTEGER NOT NULL REFERENCES connected_youtube_channels(id) ON DELETE CASCADE,
    topic TEXT NOT NULL,
    posted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    youtube_video_id VARCHAR(64)
);
CREATE INDEX IF NOT EXISTS idx_education_posted_topics_channel ON education_posted_topics(connected_channel_id);

-- Schedule configuration (admin-configurable; owner: start slow, scale later).
-- videos_per_channel_per_day is DECIMAL so 0.5 (= 1 video every 2 days) works.
INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_at)
VALUES ('education_videos_per_channel_per_day', '0.5', 'number',
        'Education videos generated+posted per connected channel per day (0.5 = 1 every 2 days). Admin-configurable up to 4.0.', NOW())
ON CONFLICT (setting_key) DO NOTHING;

-- Predefined educational topics (owner: History default; users pick from this
-- list or type custom ones). Delivered via API; no table needed, but stored
-- as a setting so admins can edit the menu without a deploy.
INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_at)
VALUES ('education_topic_menu', '["History","Personal Finance","Science Explainers","Technology","Ancient Civilizations","Space & Astronomy","Geography","Mythology","Economics","Psychology","Health & Medicine","Nature & Wildlife"]', 'json',
        'Predefined educational topics selectable in the channel topic picker. Admin-editable.', NOW())
ON CONFLICT (setting_key) DO NOTHING;
