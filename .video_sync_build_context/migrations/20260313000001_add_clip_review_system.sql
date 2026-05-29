-- Review status on clips
ALTER TABLE extracted_clips
  ADD COLUMN IF NOT EXISTS review_status VARCHAR(20) NOT NULL DEFAULT 'auto_publish',
  ADD COLUMN IF NOT EXISTS proposed_title TEXT,
  ADD COLUMN IF NOT EXISTS proposed_description TEXT,
  ADD COLUMN IF NOT EXISTS reviewed_by INTEGER REFERENCES users(id),
  ADD COLUMN IF NOT EXISTS reviewed_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS review_notes TEXT;

-- Valid values: 'auto_publish', 'pending_review', 'approved', 'rejected'

-- Per-linkage human approval toggle
ALTER TABLE youtube_channel_linkages
  ADD COLUMN IF NOT EXISTS requires_human_approval BOOLEAN NOT NULL DEFAULT false;

-- Content management agent sessions
CREATE TABLE IF NOT EXISTS content_management_sessions (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    destination_channel_id INTEGER REFERENCES connected_youtube_channels(id),
    instruction TEXT NOT NULL,
    status VARCHAR(20) NOT NULL DEFAULT 'running',  -- running, awaiting_confirmation, completed, failed
    agent_state JSONB,
    result_summary TEXT,
    confirmation_required JSONB,   -- holds pending destructive action details
    confirmation_granted BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_cms_user ON content_management_sessions(user_id, created_at DESC);

-- Audit log for destructive actions
CREATE TABLE IF NOT EXISTS content_management_audit_log (
    id SERIAL PRIMARY KEY,
    session_id INTEGER REFERENCES content_management_sessions(id),
    user_id INTEGER REFERENCES users(id),
    action VARCHAR(50) NOT NULL,
    youtube_video_id VARCHAR(255),
    clip_db_id INTEGER REFERENCES extracted_clips(id),
    details JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
