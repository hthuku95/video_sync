-- x402 paywall metadata for delivery pages.
--
-- The free /delivery/:id page renders a watermarked preview. To unlock the
-- full-quality download the visitor pays USDC on Base via the x402 protocol
-- (HTTP 402 + EIP-3009 transferWithAuthorization). Once verified, we set
-- `unlocked_until` to NOW() + 30 days and the page renders the HD download
-- buttons until that timestamp expires.
--
-- We keep the receipt id (transaction hash returned by the Coinbase
-- facilitator) for audit + refund support, and store the price in case we
-- want per-delivery pricing later (premium clients vs. samples).

ALTER TABLE deliveries
    ADD COLUMN IF NOT EXISTS unlock_price_usdc  NUMERIC(10, 2),
    ADD COLUMN IF NOT EXISTS unlocked_until     TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS payment_receipt_id TEXT;

-- Default sample-tier price ($5) for any existing rows, so the public link
-- has a price to display. Real samples for IG leads we set explicitly.
UPDATE deliveries SET unlock_price_usdc = 5.00 WHERE unlock_price_usdc IS NULL;
