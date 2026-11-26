ALTER TABLE route_batch_items
    ADD COLUMN action TEXT NOT NULL DEFAULT 'start'
    CHECK (action IN ('start', 'stop'));

CREATE TABLE route_stop_targets (
    operation_id UUID NOT NULL REFERENCES route_batch_operations(id) ON DELETE CASCADE,
    assignment_id UUID NOT NULL REFERENCES pipeline_assignments(id),
    generation BIGINT NOT NULL CHECK (generation > 0),
    PRIMARY KEY (operation_id, assignment_id)
);
