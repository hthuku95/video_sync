-- Mirror of instagram_leads.service_type for the prospects table.
-- Before this column the scorer + DM generator hardcoded a clipping pitch.
-- AI now picks one of: clipping | animations | thumbnails | ugc | full_stack
-- per prospect based on channel bio/description, and the DM is locked to
-- that one service with realistic pricing.
ALTER TABLE prospects
    ADD COLUMN IF NOT EXISTS service_type TEXT;
