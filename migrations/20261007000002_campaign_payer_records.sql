-- Payer records for all payment rails (owner directive Oct 2026).
-- PayPal: payer_email + payer_name from the order. Crypto: payer_wallet from
-- the facilitator verify response, tx hash in payment_ref. paid_amount_cents
-- freezes the charged price (grandfather-proof ledger).
ALTER TABLE campaigns ADD COLUMN IF NOT EXISTS payer_email TEXT;
ALTER TABLE campaigns ADD COLUMN IF NOT EXISTS payer_name TEXT;
ALTER TABLE campaigns ADD COLUMN IF NOT EXISTS payer_wallet TEXT;
ALTER TABLE campaigns ADD COLUMN IF NOT EXISTS payment_method TEXT;
ALTER TABLE campaigns ADD COLUMN IF NOT EXISTS paid_amount_cents INTEGER;
