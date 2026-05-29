-- Lead → delivery → payment attribution for revenue sharing.
--
-- When a whitelisted team member sources an Instagram lead, DMs them a
-- /delivery/:id sample link, and the lead pays $5 USDC to unlock HD, we
-- need to trace that tx_hash back to the user who sourced the lead so
-- we can pay them their 50% cut.
--
-- `instagram_leads.user_id` already exists (per-user scoping). This
-- migration adds funnel-stage timestamps plus a FK on `deliveries`
-- pointing back to the sourcing lead.

ALTER TABLE instagram_leads
    ADD COLUMN IF NOT EXISTS first_contacted_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS converted_at       TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_instagram_leads_converted
    ON instagram_leads(converted_at DESC NULLS LAST)
    WHERE converted_at IS NOT NULL;

-- When a delivery is created from a lead, stamp the origin so payment
-- unlocks can trace the revenue back. NULL for deliveries created
-- outside the lead workflow (direct Fiverr-style custom deliveries).
ALTER TABLE deliveries
    ADD COLUMN IF NOT EXISTS sourced_from_lead_id UUID
    REFERENCES instagram_leads(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_deliveries_sourced_from
    ON deliveries(sourced_from_lead_id)
    WHERE sourced_from_lead_id IS NOT NULL;
