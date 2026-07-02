ALTER TABLE campaigns ADD COLUMN IF NOT EXISTS source_url TEXT;

ALTER TABLE campaigns DROP CONSTRAINT IF EXISTS campaigns_service_type_check;
ALTER TABLE campaigns ADD CONSTRAINT campaigns_service_type_check
  CHECK (service_type IN (
    'clipping', 'education', 'landing_page', 'kick_auto_clipper',
    'manim_explainer', 'whiteboard_animation', 'kinetic_typography',
    'animated_infographic', 'algorithm_viz', 'investor_pitch',
    'year_in_review', 'isometric_explainer'
  ));
