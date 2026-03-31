-- Custom Deliveries table (Mar 31, 2026)
-- Stores freelancer-created custom delivery jobs for Fiverr / PPH clients.
-- Each row maps to a public /delivery/:id page the freelancer shares with the client.

CREATE TABLE IF NOT EXISTS deliveries (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    client_ref      TEXT,                   -- e.g. "Fiverr #12345 / @username"
    title           TEXT        NOT NULL,   -- shown on delivery page
    gig_type        TEXT        NOT NULL,   -- scene | thumbnail | title_card | data_viz | lower_third | latex | ui_mockup
    prompt          TEXT        NOT NULL,   -- main description / LaTeX expression / title text
    style           TEXT        NOT NULL DEFAULT 'cinematic',
    duration        FLOAT8      NOT NULL DEFAULT 10.0,
    extra_args      JSONB,                  -- gig-specific params (subtitle, title_text, device, etc.)
    status          TEXT        NOT NULL DEFAULT 'pending',  -- pending | running | completed | failed
    output_r2_url   TEXT,
    output_filename TEXT,
    error_message   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_deliveries_status     ON deliveries(status);
CREATE INDEX IF NOT EXISTS idx_deliveries_created_at ON deliveries(created_at DESC);
