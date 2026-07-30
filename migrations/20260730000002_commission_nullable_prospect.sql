-- Allow commission records without a prospect_id (e.g., for direct user payments via campaigns)
ALTER TABLE referral_commission ALTER COLUMN prospect_id DROP NOT NULL;
