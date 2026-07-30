-- Referral codes for whitelisted content machine users
CREATE TABLE referral_codes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_referral_codes_user_id ON referral_codes(user_id);
CREATE INDEX idx_referral_codes_code ON referral_codes(code);

-- Commission tracking for converted referrals
CREATE TABLE referral_commission (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    referrer_user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    prospect_id UUID NOT NULL REFERENCES prospects(id) ON DELETE CASCADE,
    deal_amount_cents INTEGER NOT NULL,
    commission_rate NUMERIC(3,2) NOT NULL DEFAULT 0.40,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'paid', 'cancelled')),
    paid_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_referral_commission_referrer ON referral_commission(referrer_user_id);

-- Add referral tracking columns to prospects
ALTER TABLE prospects ADD COLUMN IF NOT EXISTS referred_by TEXT;
ALTER TABLE prospects ADD COLUMN IF NOT EXISTS sourced_by INTEGER REFERENCES users(id) ON DELETE SET NULL;
