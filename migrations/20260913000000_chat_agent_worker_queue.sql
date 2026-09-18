-- Durable claim-based execution for chat / background agents (see §53.12).
--
-- Background agent runs (handlers/chat.rs) were fire-and-forget tokio::spawn
-- work that lived in RAM of whichever process received a WebSocket message,
-- vanished on deploy/OOM/scale events, and had unbounded concurrency. This
-- migration gives every background agent job claimable ownership semantics,
-- mirroring the agentic pipeline worker queue (migrations/20260824000000):
-- a live process claims the row (claimed_by + lease), a 60s ticker renews the
-- lease while it works, and a supervisor requeues rows whose lease expired
-- (owner died) so a reclaim worker resumes the run from its checkpoint.
--
-- Note: agent_background_jobs.status has a CHECK constraint allowing only
-- 'running' | 'completed' | 'failed' (no 'queued' state). An unclaimed job
-- is simply a row with claimed_by IS NULL; eligibility for claiming is
--   status='running' AND cancel_requested_at IS NULL
--   AND (claimed_by IS NULL OR lease_expires_at IS NULL OR lease_expires_at < NOW())

ALTER TABLE agent_background_jobs
    ADD COLUMN IF NOT EXISTS claimed_by TEXT,
    ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS attempts INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS started_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS cancel_requested_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_agent_background_jobs_claimable
    ON agent_background_jobs (status, claimed_by, lease_expires_at, created_at);