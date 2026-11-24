CREATE TABLE worker_control_sessions (
    worker_id UUID PRIMARY KEY REFERENCES workers(id) ON DELETE CASCADE,
    session_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    replaced_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX worker_control_sessions_session
    ON worker_control_sessions (session_id);
