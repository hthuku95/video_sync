-- Pipeline worker queue: durable claim-based execution for agentic workflows.
-- Replaces in-process tokio::spawn (fire-and-forget) with Postgres-backed
-- job ownership so renders survive Fargate task replacement / OOM / deploys.
--
-- claimed_by       : instance UUID of the worker currently executing (or last
--                    executing) this workflow. NULL = never claimed.
-- lease_expires_at : liveness deadline. Workers renew via heartbeat(); a
--                    supervisor sweep requeues rows whose lease expired,
--                    which makes crashed/replaced tasks' work recoverable.
-- cancel_requested_at : set by POST /api/workflows/:id/cancel; cooperative
--                    cancellation points check this between turns/tools.
--
-- All columns nullable + defaulted: zero backfill required, old rows valid.
ALTER TABLE app_workflows
    ADD COLUMN IF NOT EXISTS claimed_by TEXT,
    ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS cancel_requested_at TIMESTAMPTZ;

-- Claim timestamp for campaign posts: distinguishes "claimed moments ago,
-- delivery INSERT still in flight" from "claimed then worker died".
ALTER TABLE campaign_posts
    ADD COLUMN IF NOT EXISTS claimed_at TIMESTAMPTZ;

-- Claim query pattern: WHERE status='queued' AND workflow_type LIKE 'agentic\_%'
-- ORDER BY created_at FOR UPDATE SKIP LOCKED — index supports the hot path.
CREATE INDEX IF NOT EXISTS idx_app_workflows_queue_claim
    ON app_workflows (created_at)
    WHERE status = 'queued'
      AND workflow_type LIKE 'agentic\_%';
