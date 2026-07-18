-- Add contact_enrichment JSONB to instagram_leads for storing
-- detected_source_creators (Kick streamer slugs that this lead clips)
-- and other AI-enriched metadata.
ALTER TABLE instagram_leads
    ADD COLUMN IF NOT EXISTS contact_enrichment JSONB DEFAULT '{}'::jsonb;
