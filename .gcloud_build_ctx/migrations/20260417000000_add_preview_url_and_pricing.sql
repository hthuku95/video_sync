-- Add preview_r2_url column for watermarked preview segment (free-to-view)
-- while the clean full HD version stays behind the paywall.
ALTER TABLE deliveries ADD COLUMN IF NOT EXISTS preview_r2_url TEXT;

-- Track retry attempts for the iterative QA review system.
ALTER TABLE deliveries ADD COLUMN IF NOT EXISTS qa_retry_count INTEGER DEFAULT 0;
ALTER TABLE deliveries ADD COLUMN IF NOT EXISTS final_qa_score INTEGER;

-- Add source_url column so we can store the lead's website reference
-- for later regeneration / audit.
ALTER TABLE deliveries ADD COLUMN IF NOT EXISTS source_url TEXT;
