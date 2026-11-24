CREATE TABLE route_plan_idempotency (
    idempotency_key UUID PRIMARY KEY,
    request JSONB NOT NULL,
    response JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    CHECK (expires_at > created_at)
);

CREATE INDEX route_plan_idempotency_expiry
    ON route_plan_idempotency (expires_at);
