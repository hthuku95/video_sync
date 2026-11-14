-- Rename the retired legacy flag to reflect the Nov 13-14 2026 conversion:
-- the YouTube-API system no longer posts clips — it posts AI-generated
-- EDUCATIONAL videos to connected channels. The flag stays OFF (retired).
UPDATE service_flags SET service = 'youtube_education_posting'
WHERE service = 'legacy_youtube_clipping';
