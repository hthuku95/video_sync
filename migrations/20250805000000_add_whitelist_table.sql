-- Create whitelist_emails table
CREATE TABLE whitelist_emails (
    id SERIAL PRIMARY KEY,
    email VARCHAR(255) NOT NULL UNIQUE,
    added_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Create index for fast email lookups
CREATE INDEX idx_whitelist_emails_email ON whitelist_emails(email);

-- Create system settings table to store global configuration
CREATE TABLE system_settings (
    id SERIAL PRIMARY KEY,
    setting_key VARCHAR(100) NOT NULL UNIQUE,
    setting_value TEXT NOT NULL,
    setting_type VARCHAR(20) NOT NULL DEFAULT 'string', -- 'string', 'boolean', 'integer', 'json'
    description TEXT,
    updated_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Insert default whitelist setting (disabled by default)
INSERT INTO system_settings (setting_key, setting_value, setting_type, description) 
VALUES ('whitelist_enabled', 'false', 'boolean', 'Enable email whitelist restriction for user registration and login');

-- Create index for fast settings lookups
CREATE INDEX idx_system_settings_key ON system_settings(setting_key);

-- Add comments for documentation
COMMENT ON TABLE whitelist_emails IS 'Stores email addresses that are allowed to register/login when whitelist is enabled';
COMMENT ON TABLE system_settings IS 'Stores global system configuration settings';
COMMENT ON COLUMN system_settings.setting_type IS 'Data type of the setting value (string, boolean, integer, json)';