ALTER TABLE campaigns DROP CONSTRAINT IF EXISTS campaigns_service_type_check;
ALTER TABLE campaigns ADD CONSTRAINT campaigns_service_type_check
  CHECK (service_type IN ('clipping', 'education', 'landing_page', 'product_mockup', 'full_stack', 'kick_auto_clipper', 'business_explainer', 'voice_audio'));
