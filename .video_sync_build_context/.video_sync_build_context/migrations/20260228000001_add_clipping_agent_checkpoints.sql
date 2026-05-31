-- Add clipping_agent_checkpoints table for stateful Gemini agent resumption.
-- Stores ClippingAgentState (business logic) + Gemini conversation history after every tool call.
-- Mirrors LangGraph checkpoint pattern: crash at any point, restart resumes from the last tool.

CREATE TABLE IF NOT EXISTS clipping_agent_checkpoints (
    id               BIGSERIAL PRIMARY KEY,
    job_id           INTEGER NOT NULL REFERENCES clipping_jobs(id) ON DELETE CASCADE,
    checkpoint_num   INTEGER NOT NULL,
    phase_completed  VARCHAR(50),          -- "phase_a", "phase_b", ..., "phase_e" (nullable — mid-phase checkpoint)
    agent_state      JSONB NOT NULL,        -- ClippingAgentState (business-logic flags + phase outputs)
    gemini_history   JSONB NOT NULL,        -- Vec<Content> for full Gemini conversation reconstruction
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(job_id, checkpoint_num)
);

CREATE INDEX IF NOT EXISTS idx_clipping_agent_checkpoints_job_id
    ON clipping_agent_checkpoints(job_id);
