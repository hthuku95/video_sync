-- Samples-only batch mode (owner directive Nov 18 2026): the bulk trigger can
-- now create batches that generate sample videos ONLY — no emails are sent.
-- The owner reviews the samples first; emails can be sent later from the same
-- batch. Default 'full' preserves the original sample-first-then-email mode.
ALTER TABLE prospect_outreach_batches
    ADD COLUMN IF NOT EXISTS mode VARCHAR(12) NOT NULL DEFAULT 'full';
