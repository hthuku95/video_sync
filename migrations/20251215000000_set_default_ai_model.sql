-- Set default AI model to Gemini for cost efficiency
-- This setting can be changed by admins via the admin panel

INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_at)
VALUES ('default_ai_model', 'gemini', 'string', 'Default AI model for all users (claude or gemini)', NOW())
ON CONFLICT (setting_key)
DO UPDATE SET setting_value = 'gemini', updated_at = NOW();

-- Add comment
COMMENT ON COLUMN system_settings.setting_value IS 'For default_ai_model: claude (Sonnet 4.5) or gemini (2.5 Flash)';
