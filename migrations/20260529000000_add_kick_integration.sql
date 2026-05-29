-- Kick.com integration for source channels and three-way mapping

CREATE TABLE IF NOT EXISTS kick_source_channels (
    id SERIAL PRIMARY KEY,
    slug VARCHAR(255) NOT NULL UNIQUE,
    display_name VARCHAR(255) NOT NULL,
    profile_picture TEXT,
    is_active BOOLEAN DEFAULT true,
    broadcaster_user_id BIGINT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS youtube_kick_channel_mappings (
    id SERIAL PRIMARY KEY,
    youtube_source_channel_id INTEGER NOT NULL UNIQUE REFERENCES youtube_source_channels(id) ON DELETE CASCADE,
    kick_source_channel_id INTEGER NOT NULL REFERENCES kick_source_channels(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS twitch_kick_channel_mappings (
    id SERIAL PRIMARY KEY,
    twitch_source_channel_id INTEGER NOT NULL UNIQUE REFERENCES twitch_source_channels(id) ON DELETE CASCADE,
    kick_source_channel_id INTEGER NOT NULL REFERENCES kick_source_channels(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

ALTER TABLE youtube_source_channels
    ADD COLUMN IF NOT EXISTS kick_mapping_status VARCHAR(30) DEFAULT 'unmapped'
        CHECK (kick_mapping_status IN ('unmapped', 'mapped', 'no_kick_equivalent'));

ALTER TABLE twitch_source_channels
    ADD COLUMN IF NOT EXISTS kick_mapping_status VARCHAR(30) DEFAULT 'unmapped'
        CHECK (kick_mapping_status IN ('unmapped', 'mapped', 'no_kick_equivalent'));
