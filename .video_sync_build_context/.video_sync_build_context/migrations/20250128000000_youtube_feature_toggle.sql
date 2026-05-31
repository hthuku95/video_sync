-- Add YouTube feature toggle setting
-- This setting controls whether YouTube integration features are available to all users
-- When false: Only admins (staff/superuser) and whitelisted users have access
-- When true: All authenticated users have access

INSERT INTO system_settings (setting_key, setting_value, setting_type, description, created_at, updated_at)
VALUES (
    'youtube_features_enabled',
    'false',
    'boolean',
    'Enable YouTube integration for all users. When disabled, only admins and whitelisted users have access.',
    NOW(),
    NOW()
) ON CONFLICT (setting_key) DO NOTHING;

-- Add index for faster setting lookups (if not exists)
CREATE INDEX IF NOT EXISTS idx_system_settings_key ON system_settings(setting_key);
