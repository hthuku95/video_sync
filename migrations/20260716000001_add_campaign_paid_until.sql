ALTER TABLE campaigns ADD COLUMN IF NOT EXISTS paid_until TIMESTAMPTZ;

ALTER TABLE campaigns DROP CONSTRAINT IF EXISTS campaigns_status_check;
ALTER TABLE campaigns ADD CONSTRAINT campaigns_status_check
  CHECK (status IN ('pending_payment', 'active', 'paused', 'completed', 'cancelled'));

ALTER TABLE campaigns ALTER COLUMN status SET DEFAULT 'pending_payment';
