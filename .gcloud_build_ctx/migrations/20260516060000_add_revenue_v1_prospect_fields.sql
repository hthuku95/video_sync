-- Revenue V1: make X/email outreach and sample-pack delivery first-class
-- on cross-platform prospects.

ALTER TABLE prospects
    ADD COLUMN IF NOT EXISTS x_dm_script TEXT,
    ADD COLUMN IF NOT EXISTS email_script TEXT,
    ADD COLUMN IF NOT EXISTS sample_delivery_id UUID REFERENCES deliveries(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS contact_enrichment JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE deliveries
    ADD COLUMN IF NOT EXISTS sourced_from_prospect_id UUID REFERENCES prospects(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_prospects_revenue_v1_sort
    ON prospects (
        contact_status,
        service_type,
        ai_score DESC,
        created_at DESC
    );

CREATE INDEX IF NOT EXISTS idx_prospects_sample_delivery
    ON prospects(sample_delivery_id)
    WHERE sample_delivery_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_deliveries_sourced_from_prospect
    ON deliveries(sourced_from_prospect_id)
    WHERE sourced_from_prospect_id IS NOT NULL;
