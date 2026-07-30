-- Add workflow_id column to service_portfolio_samples for AgenticServicePipeline tracking
ALTER TABLE service_portfolio_samples ADD COLUMN IF NOT EXISTS workflow_id UUID;
