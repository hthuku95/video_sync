-- Durable, user-visible production feedback.
--
-- Redis pub/sub is used for live delivery to a running agent, but it is
-- best-effort and has no subscriber between render attempts. Persisting the
-- note means the next attempt can pick it up even if the agent was restarting
-- or the workflow was between attempts.

CREATE TABLE IF NOT EXISTS workflow_feedback (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id  UUID NOT NULL REFERENCES app_workflows(id) ON DELETE CASCADE,
    user_id      INTEGER REFERENCES users(id) ON DELETE SET NULL,
    message      TEXT NOT NULL,
    consumed_at  TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_workflow_feedback_pending
    ON workflow_feedback (workflow_id, created_at)
    WHERE consumed_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_workflow_feedback_workflow
    ON workflow_feedback (workflow_id, created_at DESC);
