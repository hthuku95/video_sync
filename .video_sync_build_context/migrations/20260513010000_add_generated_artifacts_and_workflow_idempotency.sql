ALTER TABLE app_workflows
    ADD COLUMN IF NOT EXISTS idempotency_key TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_app_workflows_idempotency_key
    ON app_workflows (idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS generated_artifacts (
    artifact_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id UUID REFERENCES app_workflows(id) ON DELETE SET NULL,
    session_uuid TEXT,
    kind TEXT NOT NULL,
    storage_backend TEXT NOT NULL,
    storage_key TEXT NOT NULL,
    file_path TEXT,
    legacy_file_id TEXT,
    public_url TEXT,
    preview_url TEXT,
    mime_type TEXT,
    bytes BIGINT,
    checksum TEXT,
    source_table TEXT,
    source_record_key TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_generated_artifacts_source
    ON generated_artifacts (source_table, source_record_key)
    WHERE source_table IS NOT NULL AND source_record_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_generated_artifacts_workflow
    ON generated_artifacts (workflow_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_generated_artifacts_session
    ON generated_artifacts (session_uuid, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_generated_artifacts_legacy_file_id
    ON generated_artifacts (legacy_file_id)
    WHERE legacy_file_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_generated_artifacts_public_url
    ON generated_artifacts (public_url)
    WHERE public_url IS NOT NULL;

CREATE OR REPLACE FUNCTION update_generated_artifacts_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_generated_artifacts_updated_at ON generated_artifacts;

CREATE TRIGGER trg_generated_artifacts_updated_at
    BEFORE UPDATE ON generated_artifacts
    FOR EACH ROW EXECUTE FUNCTION update_generated_artifacts_updated_at();
