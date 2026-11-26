CREATE TABLE route_batch_operations (
    id UUID PRIMARY KEY,
    idempotency_key UUID NOT NULL UNIQUE,
    request JSONB NOT NULL,
    response JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX route_batch_operations_created_at
    ON route_batch_operations (created_at);
