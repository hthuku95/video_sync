-- DFY customer label (Jun 22, 2026)
-- Marks users who have purchased DFY services for monetization tracking & invoicing.
-- Independent of the $15/mo DIY subscription — a user can be both, either, or neither.

ALTER TABLE users ADD COLUMN IF NOT EXISTS is_dfy_customer BOOLEAN NOT NULL DEFAULT false;

-- Index for admin filtering
CREATE INDEX IF NOT EXISTS idx_users_is_dfy_customer ON users(is_dfy_customer);
