CREATE TABLE preview_capacity_reservations (
    preview_id UUID PRIMARY KEY,
    reservation_id UUID NOT NULL UNIQUE REFERENCES worker_capacity_reservations(id),
    source_id UUID NOT NULL,
    source_revision BIGINT NOT NULL,
    worker_session_id UUID NOT NULL,
    FOREIGN KEY (source_id, source_revision)
        REFERENCES source_revisions(source_id, revision)
);

CREATE INDEX preview_capacity_reservations_source
    ON preview_capacity_reservations(source_id, source_revision);
