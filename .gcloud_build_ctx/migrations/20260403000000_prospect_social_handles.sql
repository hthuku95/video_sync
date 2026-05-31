-- Add social media handle columns to prospects table
ALTER TABLE prospects ADD COLUMN IF NOT EXISTS twitter_handle TEXT;
ALTER TABLE prospects ADD COLUMN IF NOT EXISTS discord_handle TEXT;
