-- User Zernio Profiles: maps our user IDs to Zernio profile IDs for self-service social accounts
-- Each user gets their own Zernio profile so they can connect their own social accounts

CREATE TABLE IF NOT EXISTS user_zernio_profiles (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    zernio_profile_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id),
    UNIQUE(zernio_profile_id)
);

-- Cached user social accounts (synced from Zernio so we don't need repeated API calls)
CREATE TABLE IF NOT EXISTS user_zernio_accounts (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    zernio_account_id TEXT NOT NULL,
    platform TEXT NOT NULL,
    username TEXT,
    display_name TEXT,
    profile_picture TEXT,
    is_active BOOLEAN NOT NULL DEFAULT false,
    synced_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, zernio_account_id)
);

CREATE INDEX IF NOT EXISTS idx_user_zernio_profiles_user_id ON user_zernio_profiles(user_id);
CREATE INDEX IF NOT EXISTS idx_user_zernio_accounts_user_id ON user_zernio_accounts(user_id);
