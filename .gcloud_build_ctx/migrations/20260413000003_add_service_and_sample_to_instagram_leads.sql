-- Service tag + portfolio-sample link for Instagram leads.
-- Originally these were in migration 20260413000000, but that file was
-- already applied to prod before the columns were added — modifying an
-- already-applied migration file triggers an sqlx VersionMismatch panic
-- at startup. Splitting them out into this new migration avoids that.

-- AI-picked service tag (clipping | animations | thumbnails | ugc | full_stack).
-- Set by the scorer when it judges the lead. Read by the DM generator so it
-- pitches the right service per lead instead of always pitching clipping.
ALTER TABLE instagram_leads
    ADD COLUMN IF NOT EXISTS service_type TEXT;

-- Link to a deliveries.id row that holds the auto-generated sample for THIS
-- specific lead (e.g. a Blender thumbnail or sample animation). When set,
-- the DM script can include the public /delivery/:id link as a portfolio.
ALTER TABLE instagram_leads
    ADD COLUMN IF NOT EXISTS sample_delivery_id UUID
    REFERENCES deliveries(id) ON DELETE SET NULL;
