-- Production fix: separate user conversations from system-generated run sessions.
--
-- Root cause: every pipeline service (clipping, education, campaign posts, QA retries, …)
-- auto-creates a chat_sessions row under the owning user's id so the agent can stream
-- progress into it. Those rows then appeared in the user's chat list as chats the user
-- never wrote.
--
-- Convention (enforced in conversation_manager::session_origin): pipeline services MUST
-- generate service-prefixed session UUIDs (e.g. clipping-<uuid>-try4, education-…,
-- campaign-…). Interactive chats always use plain UUIDv4 (frontend crypto.randomUUID).
-- The origin column makes that provenance queryable; user-facing lists filter to
-- origin='user'. Fail-visible by design: anything unrecognised stays visible.
ALTER TABLE chat_sessions ADD COLUMN IF NOT EXISTS origin TEXT NOT NULL DEFAULT 'user';

-- Backfill: everything not carrying a plain UUIDv4 is a machine-generated run session.
-- (≈34K rows: clipping-*, education-*, campaign-*, landing_page-*, *-tryN, test-, …)
UPDATE chat_sessions SET origin = 'system'
WHERE session_uuid !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$';

CREATE INDEX IF NOT EXISTS idx_chat_sessions_origin ON chat_sessions(user_id, origin);
