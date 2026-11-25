CREATE TABLE arc_admin_users (
    id UUID PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE arc_admin_auth_state (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    bootstrap_token_hash BYTEA NOT NULL,
    bootstrap_consumed_at TIMESTAMPTZ,
    recovery_token_hash BYTEA NOT NULL
);

CREATE TABLE arc_admin_sessions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES arc_admin_users(id) ON DELETE CASCADE,
    session_token_hash BYTEA NOT NULL UNIQUE,
    csrf_token TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ
);

CREATE INDEX arc_admin_sessions_user_active
    ON arc_admin_sessions (user_id, expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE arc_admin_login_throttles (
    username TEXT PRIMARY KEY,
    failures INTEGER NOT NULL DEFAULT 0 CHECK (failures >= 0),
    window_started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    blocked_until TIMESTAMPTZ
);
