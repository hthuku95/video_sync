INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_at)
VALUES ('auto_clipping_enabled', 'false', 'boolean',
        'Enables or disables automatic background clipping. When disabled, only manual clipping jobs are processed.',
        NOW())
ON CONFLICT (setting_key)
DO UPDATE SET setting_value = 'false', updated_at = NOW();
