-- Backfill: re-queue campaign posts that failed because their agentic workflows
-- were orphaned by a server restart (Jul 29 03:15) and the delivery recovery loop
-- could not re-trigger them (AgenticServicePipeline never stamped deliveries.workflow_id,
-- so main.rs recovery skipped them). The pipeline fix now stamps workflow_id and the
-- recovery loop also matches orphaned workflows, so resetting these posts to
-- pending_generation lets the campaign worker re-render them cleanly.

-- 1. Release the stale delivery links (their workflows are already 'failed'/orphaned).
--    Keep the deliveries but mark them failed so they don't linger as 'pending'.
UPDATE campaign_posts cp
SET status = 'pending_generation',
    error_message = NULL,
    delivery_id = NULL
WHERE cp.status = 'failed'
  AND cp.error_message IN (
    'Delivery stalled (pending >1h — pipeline may have been interrupted)',
    'Failed to start rendering'
  );

-- 2. Mark the now-orphaned deliveries as failed so they don't accumulate as pending.
UPDATE deliveries d
SET status = 'failed',
    error_message = 'Superseded by backfill — post re-queued after workflow was orphaned by restart',
    completed_at = NOW()
WHERE d.status = 'pending'
  AND NOT EXISTS (
    SELECT 1 FROM campaign_posts cp WHERE cp.delivery_id = d.id
  )
  AND d.created_at < NOW() - INTERVAL '1 hour';
