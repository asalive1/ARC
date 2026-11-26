ALTER TABLE route_batch_items
    ADD COLUMN replacement_assignment_id UUID
        REFERENCES pipeline_assignments(id);
