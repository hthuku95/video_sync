-- LinkedIn prospect fields + PhantomBuster job tracking

ALTER TABLE prospects
    ADD COLUMN IF NOT EXISTS linkedin_url        TEXT,
    ADD COLUMN IF NOT EXISTS job_title           TEXT,
    ADD COLUMN IF NOT EXISTS company_name        TEXT,
    ADD COLUMN IF NOT EXISTS company_size        TEXT,
    ADD COLUMN IF NOT EXISTS seniority_level     TEXT,
    ADD COLUMN IF NOT EXISTS email               TEXT,
    ADD COLUMN IF NOT EXISTS phantombuster_job_id TEXT;

-- PhantomBuster export jobs — tracks each Sales Navigator scrape run
CREATE TABLE IF NOT EXISTS phantombuster_jobs (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id     TEXT NOT NULL,
    agent_name   TEXT,
    search_url   TEXT,
    status       TEXT NOT NULL DEFAULT 'pending',  -- pending, running, completed, failed
    leads_found  INT  DEFAULT 0,
    error        TEXT,
    launched_at  TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
