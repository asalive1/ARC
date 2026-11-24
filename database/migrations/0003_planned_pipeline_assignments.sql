ALTER TABLE pipeline_assignments
    ADD COLUMN reservation_id UUID UNIQUE REFERENCES worker_capacity_reservations(id),
    ADD COLUMN requirements JSONB;

CREATE UNIQUE INDEX pipeline_assignments_one_planned_per_pipeline
    ON pipeline_assignments (resource_id)
    WHERE resource_type = 'pipeline' AND desired_state = 'planned';
