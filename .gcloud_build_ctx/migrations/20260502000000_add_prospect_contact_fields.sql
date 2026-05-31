-- Add creator outreach contact fields to prospects so the clipping / creator-
-- manager workflow can store business emails, Instagram handles, and websites.
ALTER TABLE prospects ADD COLUMN IF NOT EXISTS instagram_handle TEXT;
ALTER TABLE prospects ADD COLUMN IF NOT EXISTS business_email TEXT;
ALTER TABLE prospects ADD COLUMN IF NOT EXISTS external_url TEXT;
