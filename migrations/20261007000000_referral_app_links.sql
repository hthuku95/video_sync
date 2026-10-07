-- Five-app referral links (owner directive Oct 2026 §65): each whitelisted user
-- gets one link per campaign app (clips/shorts/app/learn/motion) instead of one.
-- Existing codes keep app_slug NULL = legacy "all" link, still working.
ALTER TABLE referral_codes ADD COLUMN IF NOT EXISTS app_slug TEXT;
CREATE INDEX IF NOT EXISTS idx_referral_codes_app ON referral_codes(app_slug);
-- One code per user per app (legacy NULL rows exempt).
CREATE UNIQUE INDEX IF NOT EXISTS uq_referral_codes_user_app
  ON referral_codes(user_id, app_slug) WHERE app_slug IS NOT NULL;
