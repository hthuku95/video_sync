-- Track which referral code was used when a user signed up
ALTER TABLE users ADD COLUMN IF NOT EXISTS referred_by TEXT;
-- Track which whitelisted user referred this signup (denormalized for quick lookup)
ALTER TABLE users ADD COLUMN IF NOT EXISTS referrer_user_id INTEGER REFERENCES users(id) ON DELETE SET NULL;
