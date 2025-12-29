-- Seed system_settings with model pricing (USD per 1M tokens)

-- Create system_settings table if not exists (it might be in a previous migration I missed listing, but safe to ensure)
CREATE TABLE IF NOT EXISTS system_settings (
    id SERIAL PRIMARY KEY,
    setting_key VARCHAR(255) UNIQUE NOT NULL,
    setting_value TEXT NOT NULL,
    setting_type VARCHAR(50) NOT NULL, -- 'string', 'boolean', 'integer', 'decimal'
    description TEXT,
    updated_by INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Insert or Update Claude 3.5 Sonnet Pricing
INSERT INTO system_settings (setting_key, setting_value, setting_type, description)
VALUES 
    ('model_pricing.claude-3-5-sonnet-latest.input', '3.00', 'decimal', 'Cost per 1M input tokens for Claude 3.5 Sonnet'),
    ('model_pricing.claude-3-5-sonnet-latest.output', '15.00', 'decimal', 'Cost per 1M output tokens for Claude 3.5 Sonnet')
ON CONFLICT (setting_key) DO UPDATE 
SET setting_value = EXCLUDED.setting_value, updated_at = NOW();

-- Insert or Update Gemini 3.0 Pro Pricing (Updated based on latest docs/search)
INSERT INTO system_settings (setting_key, setting_value, setting_type, description)
VALUES 
    ('model_pricing.gemini-3-pro-preview.input', '2.00', 'decimal', 'Cost per 1M input tokens for Gemini 3.0 Pro (Standard)'),
    ('model_pricing.gemini-3-pro-preview.output', '12.00', 'decimal', 'Cost per 1M output tokens for Gemini 3.0 Pro (Standard)')
ON CONFLICT (setting_key) DO UPDATE 
SET setting_value = EXCLUDED.setting_value, updated_at = NOW();