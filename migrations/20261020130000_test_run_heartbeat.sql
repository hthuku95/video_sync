-- Resumable portfolio test runs.
--
-- A run is driven by a tokio task that lives and dies with the Fargate task.
-- When a deploy (or OOM, or panic) kills that task, the run's row was left
-- status='running' forever — observed 14h with zero agent activity on Oct 20.
--
-- Two additions make the run resumable instead of orphaned:
--   1. heartbeat_at: the driving task pings every RUN_HEARTBEAT_SECS. A run
--      whose heartbeat is older than RUN_STALE_AFTER_SECS has no live driver,
--      so any replica may claim and resume it. A live run on a healthy replica
--      keeps its heartbeat fresh and is never stolen.
--   2. test_results.delivery_id: the delivery backing each scenario. Its
--      workflow is owned by the durable pipeline_worker lease system, so a
--      resumed run reattaches to the still-running render (or reads its
--      finished output) instead of paying for a second render.

ALTER TABLE test_runs ADD COLUMN IF NOT EXISTS heartbeat_at TIMESTAMPTZ;

COMMENT ON COLUMN test_runs.heartbeat_at IS
  'Liveness ping from the task driving this run. A running run whose heartbeat is older than ~2 minutes is orphaned and may be resumed by any replica.';

ALTER TABLE test_results ADD COLUMN IF NOT EXISTS delivery_id UUID;

COMMENT ON COLUMN test_results.delivery_id IS
  'Delivery created for this scenario. Used to reattach to the durable workflow after an interruption instead of re-rendering.';

CREATE INDEX IF NOT EXISTS idx_test_runs_running_heartbeat
    ON test_runs (heartbeat_at)
    WHERE status = 'running';

-- Backfill: existing running rows predate heartbeats and have no live driver.
-- Seed from started_at so they are immediately eligible for resume.
UPDATE test_runs
   SET heartbeat_at = started_at
 WHERE status = 'running'
   AND heartbeat_at IS NULL;
