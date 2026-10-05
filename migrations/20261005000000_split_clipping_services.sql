-- Split the legacy `clipping` managed service into platform-specific services
-- (owner directive Oct 2026): `youtube_clipping` (OFF at launch — YouTube
-- downloads are bot-walled, no Apify budget) + `twitch_clipping` (ON at launch
-- alongside kick_auto_clipper). 12 -> 13 managed services; three Zernio-powered
-- clipping services: kick_auto_clipper, youtube_clipping, twitch_clipping.
--
-- Legacy `clipping` rows keep working: the flag row stays (switched OFF, so old
-- campaigns park at zero LLM cost) and ServiceType::from_normalized still maps
-- it to the shared clipping core.

INSERT INTO service_flags (service, enabled, updated_by, updated_at)
VALUES
  ('youtube_clipping', FALSE, 'seed-split-oct5', NOW()),
  ('twitch_clipping', TRUE, 'seed-split-oct5', NOW())
ON CONFLICT (service) DO NOTHING;

-- Legacy `clipping` is YouTube-dominant: park it OFF with youtube_clipping.
-- (Idempotent: only flips rows the seed created as ON.)
UPDATE service_flags SET enabled = FALSE, updated_at = NOW()
WHERE service = 'clipping' AND enabled = TRUE;

-- Widen the campaigns CHECK to admit the two new service keys.
ALTER TABLE campaigns DROP CONSTRAINT IF EXISTS campaigns_service_type_check;
ALTER TABLE campaigns ADD CONSTRAINT campaigns_service_type_check
  CHECK (service_type IN (
    'clipping', 'education', 'landing_page', 'kick_auto_clipper',
    'manim_explainer', 'whiteboard_animation', 'kinetic_typography',
    'animated_infographic', 'algorithm_viz', 'investor_pitch',
    'year_in_review', 'isometric_explainer',
    'youtube_clipping', 'twitch_clipping'
  ));
