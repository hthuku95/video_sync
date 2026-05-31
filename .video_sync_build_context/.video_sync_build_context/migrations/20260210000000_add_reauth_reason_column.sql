-- Add missing reauth_reason column to connected_youtube_channels
-- This column should have been added in 20250101000000 but was omitted
-- Migration: 20260210000000_add_reauth_reason_column.sql

-- Add reauth_reason column (idempotent - safe to run multiple times)
ALTER TABLE connected_youtube_channels
ADD COLUMN IF NOT EXISTS reauth_reason TEXT;

-- Add index for filtering channels by reauth status
CREATE INDEX IF NOT EXISTS idx_youtube_channels_requires_reauth
ON connected_youtube_channels(requires_reauth)
WHERE requires_reauth = true;

-- Add comment explaining the field
COMMENT ON COLUMN connected_youtube_channels.reauth_reason IS
'Human-readable reason why channel requires re-authentication (e.g., "Token expired", "Insufficient scopes", "User revoked access")';

-- Set default reason for existing channels that require reauth
UPDATE connected_youtube_channels
SET reauth_reason = 'Token refresh failed - please reconnect'
WHERE requires_reauth = true AND reauth_reason IS NULL;
