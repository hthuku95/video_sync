-- Scope Instagram leads + PhantomBuster jobs per user.
-- Previously every lead went into a shared pool — every user saw the same
-- dashboard. Users want private pipelines: each user runs their own searches,
-- owns their own leads, and can't see another user's DMs or contact statuses.

-- ────────────────────────────────────────────────────────────────────────────
-- phantombuster_jobs.user_id — who launched the PB run
-- Nullable for the existing rows (launched before this column existed).
-- ────────────────────────────────────────────────────────────────────────────
ALTER TABLE phantombuster_jobs
    ADD COLUMN IF NOT EXISTS user_id INT
    REFERENCES users(id) ON DELETE CASCADE;

CREATE INDEX IF NOT EXISTS idx_phantombuster_jobs_user_id
    ON phantombuster_jobs(user_id);

-- ────────────────────────────────────────────────────────────────────────────
-- instagram_leads.user_id — who owns the lead.
-- Drop the global UNIQUE(username) and replace with UNIQUE(user_id, username)
-- so two different users can independently have the same creator in their
-- pipelines. Within a single user's pool usernames are still unique so the
-- ON CONFLICT upsert keeps working.
-- ────────────────────────────────────────────────────────────────────────────
ALTER TABLE instagram_leads
    ADD COLUMN IF NOT EXISTS user_id INT
    REFERENCES users(id) ON DELETE CASCADE;

-- Find and drop the old unique index on username. Postgres names it based on
-- the table/column unless we named it explicitly — so look it up dynamically.
DO $$
DECLARE
    constraint_name TEXT;
BEGIN
    SELECT conname INTO constraint_name
    FROM pg_constraint
    WHERE conrelid = 'instagram_leads'::regclass
      AND contype  = 'u'
      AND pg_get_constraintdef(oid) = 'UNIQUE (username)';
    IF constraint_name IS NOT NULL THEN
        EXECUTE format('ALTER TABLE instagram_leads DROP CONSTRAINT %I', constraint_name);
    END IF;
END$$;

-- New composite unique: within one user, username is unique.
-- Allows the same creator to appear in multiple users' pipelines.
CREATE UNIQUE INDEX IF NOT EXISTS idx_instagram_leads_user_username
    ON instagram_leads(user_id, username);

CREATE INDEX IF NOT EXISTS idx_instagram_leads_user_id
    ON instagram_leads(user_id);

-- Backfill: stamp existing leads/jobs with the original superadmin user so
-- they don't spontaneously vanish from the dashboard. Pick the oldest active
-- superuser; if there isn't one, leave NULL (worker will ignore NULL rows
-- when showing the user dashboard).
DO $$
DECLARE
    admin_id INT;
BEGIN
    SELECT id INTO admin_id FROM users
    WHERE is_superuser = TRUE AND is_active = TRUE
    ORDER BY created_at ASC LIMIT 1;

    IF admin_id IS NOT NULL THEN
        UPDATE instagram_leads       SET user_id = admin_id WHERE user_id IS NULL;
        UPDATE phantombuster_jobs    SET user_id = admin_id WHERE user_id IS NULL;
    END IF;
END$$;
