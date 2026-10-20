-- Cross-replica pacing for NVIDIA NIM.
--
-- The NIM free tier throttles the account, not the process. Each of the three
-- Fargate tasks pacing itself to one in-flight request still produced bursts
-- of 429s (2026-10-20: every tool turn retried twice before succeeding).
-- A single row holding the next allowed request time lets every replica
-- reserve a slot atomically and sleep until then.
CREATE TABLE IF NOT EXISTS nim_rate_limit (
    id      SMALLINT PRIMARY KEY,
    next_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO nim_rate_limit (id) VALUES (1)
ON CONFLICT (id) DO NOTHING;
