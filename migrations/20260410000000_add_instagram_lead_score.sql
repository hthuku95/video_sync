-- Add AI scoring columns to instagram_leads
ALTER TABLE instagram_leads
    ADD COLUMN IF NOT EXISTS score       INT,          -- 0–100 likelihood of being a paying client
    ADD COLUMN IF NOT EXISTS score_reason TEXT;         -- AI explanation

CREATE INDEX IF NOT EXISTS instagram_leads_score_idx ON instagram_leads(score DESC NULLS LAST);
