CREATE TABLE capacity_wait_requests (
    sequence BIGSERIAL PRIMARY KEY,
    request_id UUID NOT NULL UNIQUE,
    idempotency_key UUID NOT NULL UNIQUE,
    pipeline_id UUID NOT NULL,
    source_id UUID NOT NULL,
    source_revision BIGINT NOT NULL,
    destination_id UUID NOT NULL,
    destination_revision BIGINT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('waiting', 'planned', 'unavailable', 'cancelled')),
    estimate JSONB NOT NULL,
    result JSONB,
    reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT (clock_timestamp() + INTERVAL '5 minutes')
);

CREATE INDEX capacity_wait_fifo ON capacity_wait_requests (sequence) WHERE state = 'waiting';
CREATE UNIQUE INDEX capacity_wait_active_pipeline ON capacity_wait_requests (pipeline_id)
    WHERE state = 'waiting';

CREATE FUNCTION notify_capacity_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_notify('arc_capacity_change', '');
    RETURN NULL;
END;
$$;

CREATE TRIGGER capacity_workers_changed AFTER INSERT OR UPDATE OR DELETE ON workers
    FOR EACH ROW EXECUTE FUNCTION notify_capacity_change();
CREATE TRIGGER capacity_reservations_changed AFTER INSERT OR UPDATE OR DELETE ON worker_capacity_reservations
    FOR EACH ROW EXECUTE FUNCTION notify_capacity_change();
CREATE TRIGGER capacity_wait_requests_changed AFTER INSERT OR UPDATE OR DELETE ON capacity_wait_requests
    FOR EACH ROW EXECUTE FUNCTION notify_capacity_change();
