-- Per-workload queue priorities for the durable pipeline runner.
-- Lower number = claimed sooner. Prevents bulk sample/portfolio generation
-- from starving time-sensitive work (scheduled campaign posts, clipping).
--
-- Priority bands (assigned at enqueue by AgenticServicePipeline::start):
--   50  time-sensitive: campaign posts + admin/clipping deliveries
--   100 default
--   200 bulk/backfillable: portfolio tests, service samples, gig samples
ALTER TABLE app_workflows
    ADD COLUMN IF NOT EXISTS priority SMALLINT NOT NULL DEFAULT 100;
