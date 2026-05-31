-- Add token usage and cost tracking to conversation_messages table

ALTER TABLE conversation_messages
ADD COLUMN IF NOT EXISTS prompt_tokens INTEGER,
ADD COLUMN IF NOT EXISTS completion_tokens INTEGER,
ADD COLUMN IF NOT EXISTS total_tokens INTEGER,
ADD COLUMN IF NOT EXISTS model VARCHAR(50),
ADD COLUMN IF NOT EXISTS cost_usd DECIMAL(10, 6);

-- Create index for analytics on cost/usage
CREATE INDEX IF NOT EXISTS idx_conversation_messages_model ON conversation_messages(model);
