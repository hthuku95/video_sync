ALTER TABLE deliveries ADD COLUMN IF NOT EXISTS zernio_profile_id TEXT;
ALTER TABLE deliveries ADD COLUMN IF NOT EXISTS zernio_account_ids JSONB DEFAULT '[]'::jsonb;
