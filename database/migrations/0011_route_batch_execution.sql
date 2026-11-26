ALTER TABLE route_batch_operations
    ADD COLUMN execution_state TEXT NOT NULL DEFAULT 'desired_state_applied'
        CHECK (execution_state IN (
            'desired_state_applied',
            'pending',
            'running',
            'completed',
            'partial',
            'failed'
        ));

CREATE TABLE route_batch_items (
    operation_id UUID NOT NULL REFERENCES route_batch_operations(id) ON DELETE CASCADE,
    source_id UUID NOT NULL REFERENCES sources(id),
    destination_id UUID NOT NULL REFERENCES destinations(id),
    source_revision BIGINT NOT NULL CHECK (source_revision > 0),
    destination_revision BIGINT NOT NULL CHECK (destination_revision > 0),
    route_revision BIGINT NOT NULL CHECK (route_revision > 0),
    pipeline_id UUID NOT NULL UNIQUE,
    planning_key UUID NOT NULL UNIQUE,
    state TEXT NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'processing', 'succeeded', 'failed')),
    assignment_id UUID,
    assignment_generation BIGINT,
    error_code TEXT,
    error_message TEXT,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    lease_expires_at TIMESTAMPTZ,
    lease_token UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (operation_id, destination_id),
    CHECK ((state = 'processing') = (lease_expires_at IS NOT NULL)),
    CHECK ((state = 'processing') = (lease_token IS NOT NULL))
);

CREATE INDEX route_batch_items_ready
    ON route_batch_items (next_attempt_at, created_at)
    WHERE state = 'pending';

CREATE INDEX route_batch_items_expired_lease
    ON route_batch_items (lease_expires_at)
    WHERE state = 'processing';
