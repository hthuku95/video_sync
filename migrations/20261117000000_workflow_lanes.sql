-- Workflow lanes (owner design Nov 16 2026): interactive = someone is waiting
-- (paying customers, prospecting samples, admin tests); background = scheduled
-- batch (education-posting network, future Zernio campaigns). Claimers filter
-- by lane: vs-worker-1 serves interactive, vs-edu-worker serves background.
ALTER TABLE app_workflows ADD COLUMN IF NOT EXISTS lane VARCHAR(16) NOT NULL DEFAULT 'interactive';
CREATE INDEX IF NOT EXISTS idx_app_workflows_lane ON app_workflows(lane, status);

-- Backfill: education-posting workflows are BACKGROUND lane (scheduled batch
-- for the YouTube network). Everything else defaults to interactive.
UPDATE app_workflows
   SET lane = 'background'
 WHERE workflow_type = 'agentic_education'
   AND source_record_id IN (
       SELECT d.id FROM deliveries d WHERE d.client_ref LIKE 'education_post:%'
   )
   AND lane <> 'background';
