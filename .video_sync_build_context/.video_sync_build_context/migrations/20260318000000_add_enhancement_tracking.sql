-- Add Phase C+ enhancement tracking columns to extracted_clips
ALTER TABLE extracted_clips
    ADD COLUMN IF NOT EXISTS enhancement_applied   BOOLEAN  NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS enhancement_tools     TEXT[]   NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS enhancement_reasoning TEXT;
