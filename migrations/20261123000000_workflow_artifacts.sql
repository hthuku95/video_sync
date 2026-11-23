-- Artifact-level checkpointing: each stage of a workflow persists its output
-- so retries can resume from the last successful stage instead of starting over.
CREATE TABLE IF NOT EXISTS workflow_artifacts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id UUID NOT NULL REFERENCES app_workflows(id) ON DELETE CASCADE,
    stage TEXT NOT NULL,          -- 'crawl', 'script', 'narration', 'composition', 'render', 'audio_mux', 'review', 'final'
    artifact_type TEXT NOT NULL,  -- 'text', 'json', 'audio_url', 'html', 'video_url'
    content TEXT,                -- script text, HyperFrames HTML/JS, timeline JSON, review feedback
    storage_url TEXT,            -- B2/R2 URL for audio/video files (NULL for text/html)
    storage_backend TEXT,        -- 'b2', 'r2', 's3'
    metadata JSONB DEFAULT '{}', -- duration, score, feature list, etc.
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(workflow_id, stage)    -- one artifact per stage per workflow
);

CREATE INDEX IF NOT EXISTS idx_workflow_artifacts_workflow ON workflow_artifacts(workflow_id);
CREATE INDEX IF NOT EXISTS idx_workflow_artifacts_stage ON workflow_artifacts(workflow_id, stage);
