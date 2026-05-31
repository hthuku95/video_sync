-- Portfolio test runner tables (Mar 28, 2026)
-- Stores Fiverr gig scenario test runs and individual results,
-- including R2 output URLs and Gemini LLM review scores.

CREATE TABLE IF NOT EXISTS test_runs (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT        NOT NULL,
    status          TEXT        NOT NULL DEFAULT 'running',  -- running | completed | failed
    started_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ,
    total_tests     INT         NOT NULL DEFAULT 0,
    passed_tests    INT         NOT NULL DEFAULT 0,
    failed_tests    INT         NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS test_results (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id              UUID        NOT NULL REFERENCES test_runs(id) ON DELETE CASCADE,
    test_name           TEXT        NOT NULL,
    gig_type            TEXT        NOT NULL,
    prompt              TEXT        NOT NULL,
    status              TEXT        NOT NULL DEFAULT 'pending', -- pending | running | passed | failed
    output_r2_key       TEXT,
    output_r2_url       TEXT,
    output_filename     TEXT,
    error_message       TEXT,
    llm_review_score    INT,
    llm_review_feedback TEXT,
    llm_reviewer        TEXT        DEFAULT 'gemini',
    started_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at        TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_test_results_run_id ON test_results(run_id);
CREATE INDEX IF NOT EXISTS idx_test_runs_status    ON test_runs(status);
CREATE INDEX IF NOT EXISTS idx_test_runs_started   ON test_runs(started_at DESC);
