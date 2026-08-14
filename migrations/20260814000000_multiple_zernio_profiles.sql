-- One-to-many: a user can own multiple Zernio profiles.
-- Each profile can hold up to N connected accounts (currently 2 per profile).
-- Drop the UNIQUE(user_id) constraint so the same user can map to many profiles.
-- Keep UNIQUE(zernio_profile_id) so a Zernio profile maps to at most one user.

ALTER TABLE user_zernio_profiles DROP CONSTRAINT IF EXISTS user_zernio_profiles_user_id_key;

-- Display name for the profile (e.g. "hthuku — VideoSync", "hthuku — VideoSync #2").
ALTER TABLE user_zernio_profiles ADD COLUMN IF NOT EXISTS name TEXT;

-- Track which profile each cached account belongs to so we can group by profile.
ALTER TABLE user_zernio_accounts ADD COLUMN IF NOT EXISTS zernio_profile_id TEXT;
