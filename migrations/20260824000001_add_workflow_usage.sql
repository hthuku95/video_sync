-- Per-run usage ledger rollup for agentic workflows.
-- Written every agent turn by RunLedger::flush (stateful_agent.rs); read by
-- the admin trace endpoint. Overwrite-merge semantics via jsonb concatenation
-- of a top-level "usage" key.
ALTER TABLE app_workflows
    ADD COLUMN IF NOT EXISTS usage JSONB;
