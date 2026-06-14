-- Service Portfolio Samples: agent-generated demo outputs per DFY service
CREATE TABLE IF NOT EXISTS service_portfolio_samples (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    service_slug        TEXT        NOT NULL,
    sample_name         TEXT        NOT NULL,
    brief               TEXT        NOT NULL,
    description         TEXT        NOT NULL DEFAULT '',
    status              TEXT        NOT NULL DEFAULT 'pending',
    session_id          TEXT,
    output_r2_url       TEXT,
    output_thumbnail_url TEXT,
    llm_review_score    INT,
    llm_review_feedback TEXT,
    error_message       TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at          TIMESTAMPTZ,
    completed_at        TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_portfolio_samples_service_slug ON service_portfolio_samples(service_slug);
