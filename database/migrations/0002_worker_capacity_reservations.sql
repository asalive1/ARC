CREATE TABLE worker_capacity_reservations (
    id UUID PRIMARY KEY,
    pipeline_id UUID NOT NULL,
    worker_id UUID NOT NULL REFERENCES workers(id),
    capacity_units BIGINT NOT NULL CHECK (capacity_units BETWEEN 1 AND 4294967295),
    state TEXT NOT NULL CHECK (state IN ('reserved', 'released')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_at TIMESTAMPTZ,
    CHECK ((state = 'reserved' AND released_at IS NULL)
        OR (state = 'released' AND released_at IS NOT NULL))
);

CREATE UNIQUE INDEX worker_capacity_reservations_one_active_per_pipeline
    ON worker_capacity_reservations (pipeline_id)
    WHERE state = 'reserved';

CREATE INDEX worker_capacity_reservations_active_worker
    ON worker_capacity_reservations (worker_id)
    WHERE state = 'reserved';
