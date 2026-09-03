-- Execution-age tracking for the agentic pipeline age-cap.
--
-- requeue_expired_leases fails runs older than AGENTIC_MAX_RUN_HOURS. That
-- age must measure EXECUTION time (from first claim), not queue wait — a
-- workflow queued 7h behind a backlog hasn't "run" for 7h. The claim query
-- stamps started_at on first claim; the age-cap uses
-- COALESCE(started_at, created_at).
--
-- Deployed live via psql on Sep 3 2026 (td :12 supervisor was failing every
-- sweep on the missing column); this migration makes it reproducible.

ALTER TABLE app_workflows ADD COLUMN IF NOT EXISTS started_at TIMESTAMPTZ;

UPDATE app_workflows SET started_at = created_at WHERE started_at IS NULL;
