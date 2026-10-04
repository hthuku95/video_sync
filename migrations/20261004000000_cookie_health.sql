-- YouTube cookie health for the ytdlp download pipeline (owner directive Oct 2026).
--
-- Cookie exports rot (Google rotates sessions; foreign-IP reuse accelerates
-- it). When downloads start failing, the admin must re-export fresh cookies.
-- This table drives automated email reminders so staleness is caught by the
-- system instead of by failed renders:
--   * consecutive_failures: reset on every successful R2 source download,
--     incremented on every download-pipeline failure.
--   * Failure alert: >= threshold with 24h cooldown → email admin.
--   * Age alert: last_deployed_at older than max age with 7d cooldown.
-- Single row (id=1). Updated by the pipeline itself + the admin endpoint.
CREATE TABLE IF NOT EXISTS ytdlp_cookie_health (
  id INT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
  last_deployed_at TIMESTAMPTZ,
  deployed_by TEXT,
  consecutive_failures INT NOT NULL DEFAULT 0,
  last_reminded_at TIMESTAMPTZ
);
INSERT INTO ytdlp_cookie_health (id) VALUES (1) ON CONFLICT DO NOTHING;
