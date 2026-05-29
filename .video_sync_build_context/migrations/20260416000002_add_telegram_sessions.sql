-- MTProto session storage for the Telegram userbot.
-- Only one active session at a time (single_session = TRUE uniqueness
-- guaranteed by partial unique index). `session_blob` holds the raw
-- grammers session bytes — serializes auth state so restarts don't
-- require a new phone-code login.
--
-- Phase 1 of the Telegram opportunity watcher (manual paste) already
-- landed. Phase 2 = this — once the admin completes login from the
-- /admin/prospect-finder UI, the watcher starts polling channels listed
-- in telegram_watch_channels and inserts matches into
-- telegram_opportunities automatically.

CREATE TABLE IF NOT EXISTS telegram_sessions (
    id            SERIAL PRIMARY KEY,
    phone         TEXT NOT NULL,
    -- Opaque grammers session bytes. Contains the auth key + DC info.
    -- Treat as a password — anyone with this can impersonate the account.
    session_blob  BYTEA,
    -- Login handshake state kept between /login/start and /login/verify.
    -- Null after verify succeeds.
    phone_code_hash TEXT,
    authorized    BOOLEAN NOT NULL DEFAULT FALSE,
    last_poll_at  TIMESTAMPTZ,
    last_error    TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Only one active authorized session. Pending logins can stack up (a
-- user might start the flow and abandon); we always use the most recent
-- authorized row.
CREATE INDEX IF NOT EXISTS idx_telegram_sessions_authorized
    ON telegram_sessions(authorized, created_at DESC)
    WHERE authorized = TRUE;
