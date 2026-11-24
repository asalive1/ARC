ALTER TABLE pipeline_assignments
    ADD COLUMN source_id UUID,
    ADD COLUMN source_revision BIGINT,
    ADD COLUMN destination_id UUID,
    ADD COLUMN destination_revision BIGINT,
    ADD COLUMN media_path TEXT,
    ADD COLUMN route_configuration JSONB;

ALTER TABLE pipeline_assignments
    ADD CONSTRAINT pipeline_assignments_source_revision_fk
        FOREIGN KEY (source_id, source_revision)
        REFERENCES source_revisions (source_id, revision),
    ADD CONSTRAINT pipeline_assignments_destination_revision_fk
        FOREIGN KEY (destination_id, destination_revision)
        REFERENCES destination_revisions (destination_id, revision),
    ADD CONSTRAINT pipeline_assignments_route_snapshot_check
        CHECK (
            (source_id IS NULL AND source_revision IS NULL
                AND destination_id IS NULL AND destination_revision IS NULL
                AND media_path IS NULL AND route_configuration IS NULL)
            OR
            (source_id IS NOT NULL AND source_revision IS NOT NULL AND source_revision > 0
                AND destination_id IS NOT NULL
                AND destination_revision IS NOT NULL AND destination_revision > 0
                AND media_path IS NOT NULL AND route_configuration IS NOT NULL)
        );

CREATE INDEX pipeline_assignments_source_revision_lookup
    ON pipeline_assignments (source_id, source_revision)
    WHERE source_id IS NOT NULL;
CREATE INDEX pipeline_assignments_destination_revision_lookup
    ON pipeline_assignments (destination_id, destination_revision)
    WHERE destination_id IS NOT NULL;
