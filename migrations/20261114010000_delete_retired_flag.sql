-- The retired legacy flag (formerly legacy_youtube_clipping, renamed
-- youtube_education_posting on Nov 14 2026) does nothing — deleted so the
-- admin switches page shows only flags with a real function.
DELETE FROM service_flags WHERE service IN ('youtube_education_posting', 'legacy_youtube_clipping');
