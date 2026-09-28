-- Runtime on/off switches for AI-powered services (owner directive Sep 2026).
--
-- Every entry point that spends LLM tokens (pipeline renders, campaign slots,
-- prospect scoring, sample packs, chat agents, legacy clipping workers) checks
-- service_flags before starting. In-run calls (QA review, embeddings, skill
-- writes) inherit permission from the run — no separate flags to misconfigure.
-- Admin UI + POST /api/admin/service-flags flip rows at runtime (no redeploy).
-- Missing rows default to enabled (fail-open for unlisted legacy gig types).
CREATE TABLE IF NOT EXISTS service_flags (
  service TEXT PRIMARY KEY,
  enabled BOOLEAN NOT NULL DEFAULT TRUE,
  updated_by TEXT,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed: only the two Zernio-powered clipping businesses run.
INSERT INTO service_flags (service, enabled, updated_by) VALUES
  ('clipping', TRUE, 'seed'),
  ('kick_auto_clipper', TRUE, 'seed'),
  ('landing_page', FALSE, 'seed'),
  ('education', FALSE, 'seed'),
  ('manim_explainer', FALSE, 'seed'),
  ('whiteboard_animation', FALSE, 'seed'),
  ('kinetic_typography', FALSE, 'seed'),
  ('animated_infographic', FALSE, 'seed'),
  ('algorithm_viz', FALSE, 'seed'),
  ('investor_pitch', FALSE, 'seed'),
  ('year_in_review', FALSE, 'seed'),
  ('isometric_explainer', FALSE, 'seed'),
  ('prospecting', TRUE, 'seed'),
  ('outreach', TRUE, 'seed'),
  ('sample_packs', TRUE, 'seed'),
  ('legacy_youtube_clipping', FALSE, 'seed'),
  ('chat_agents', FALSE, 'seed'),
  ('admin_tests', TRUE, 'seed')
ON CONFLICT (service) DO NOTHING;
