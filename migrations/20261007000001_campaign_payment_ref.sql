-- Campaign payment receipt (owner directive Oct 2026 §65): store the PayPal
-- order id or x402 tx hash that activated the campaign. Prevents double-spend
-- of one PayPal order across multiple campaigns.
ALTER TABLE campaigns ADD COLUMN IF NOT EXISTS payment_ref TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS uq_campaigns_payment_ref
  ON campaigns(payment_ref) WHERE payment_ref IS NOT NULL;
