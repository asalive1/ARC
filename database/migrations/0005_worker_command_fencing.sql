CREATE TABLE worker_assignment_fences (
    pipeline_id UUID PRIMARY KEY,
    worker_id UUID NOT NULL REFERENCES workers(id),
    assignment_id UUID NOT NULL REFERENCES pipeline_assignments(id),
    generation BIGINT NOT NULL CHECK (generation > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE worker_command_ledger (
    command_id UUID PRIMARY KEY,
    worker_id UUID NOT NULL REFERENCES workers(id),
    pipeline_id UUID NOT NULL,
    assignment_id UUID NOT NULL REFERENCES pipeline_assignments(id),
    generation BIGINT NOT NULL CHECK (generation > 0),
    status TEXT NOT NULL CHECK (status IN ('processing', 'completed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    completed_at TIMESTAMPTZ,
    CHECK (
        (status = 'processing' AND completed_at IS NULL)
        OR (status = 'completed' AND completed_at IS NOT NULL)
    )
);

CREATE INDEX worker_command_ledger_pipeline_generation
    ON worker_command_ledger (pipeline_id, generation);
