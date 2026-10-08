-- Multi-sender email log (owner directive Oct 2026): record which
-- @videosync.ink address each email was sent from.
ALTER TABLE email_log ADD COLUMN IF NOT EXISTS from_email TEXT;
