CREATE TABLE preview_worker_process_fences (
    worker_id UUID PRIMARY KEY REFERENCES workers(id) ON DELETE CASCADE,
    incarnation_id UUID NOT NULL UNIQUE,
    process_identity JSONB NOT NULL
);

ALTER TABLE worker_control_sessions ADD COLUMN process_incarnation UUID;

CREATE FUNCTION check_preview_worker_process_incarnation() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    expected UUID;
BEGIN
    PERFORM id FROM workers WHERE id = NEW.worker_id FOR KEY SHARE;
    SELECT incarnation_id INTO expected FROM preview_worker_process_fences
        WHERE worker_id = NEW.worker_id FOR KEY SHARE;
    IF FOUND AND NEW.process_incarnation IS DISTINCT FROM expected THEN
        RAISE EXCEPTION 'worker process incarnation does not match its fence'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER worker_control_process_fence
    BEFORE INSERT OR UPDATE ON worker_control_sessions
    FOR EACH ROW EXECUTE FUNCTION check_preview_worker_process_incarnation();
