-- Runtime on/off switches for customer-facing payment methods (owner directive Oct 2026).
-- PayPal/Card stays ON; USDC and any others default OFF until explicitly enabled.
-- Consumed by GET /api/payment-methods (public) and the admin toggle page.

CREATE TABLE IF NOT EXISTS payment_methods (
    method      TEXT PRIMARY KEY,
    enabled     BOOLEAN NOT NULL DEFAULT FALSE,
    updated_by  TEXT,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO payment_methods (method, enabled, updated_by) VALUES
    ('paypal', TRUE, 'seed'),
    ('usdc',   FALSE, 'seed')
ON CONFLICT (method) DO NOTHING;
