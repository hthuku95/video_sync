-- Durable agent runs: turn-boundary checkpoint for the agentic tool loop.
-- The agent loop serializes its full message history after every completed
-- model+tool exchange. On process crash / task recycle / provider timeout,
-- the next attempt resumes from the last good turn instead of restarting.
-- Cleared on successful completion. NULL = no checkpoint (fresh start).
ALTER TABLE app_workflows
    ADD COLUMN IF NOT EXISTS agent_checkpoint JSONB;
