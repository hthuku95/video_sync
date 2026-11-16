-- Fair-rotation upload budget (owner design Nov 15-16 2026): the YouTube Data
-- API quota caps uploads project-wide (10,000 units/day; 1,600 per upload →
-- ~6 uploads/day). The scheduler posts to a rotating subset of channels
-- (oldest-posted-first) so distribution stays fair; this setting bounds the
-- daily uploads. Raise when the quota increase is approved.
INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_at)
VALUES ('education_daily_upload_budget', '6', 'number',
        'Max education video uploads per rolling 24h (quota-driven; ~6 fits the default 10,000-unit YouTube API quota). Raise when quota increases.', NOW())
ON CONFLICT (setting_key) DO NOTHING;
