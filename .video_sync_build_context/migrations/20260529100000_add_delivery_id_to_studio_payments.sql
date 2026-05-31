-- Link studio payments to deliveries for PayPal/USDC fulfillment
ALTER TABLE studio_payments
    ADD COLUMN IF NOT EXISTS delivery_id UUID REFERENCES deliveries(id);
