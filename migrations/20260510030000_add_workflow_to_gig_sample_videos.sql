ALTER TABLE gig_sample_videos
ADD COLUMN IF NOT EXISTS workflow_id UUID REFERENCES app_workflows(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_gig_sample_videos_workflow_id
    ON gig_sample_videos(workflow_id);
