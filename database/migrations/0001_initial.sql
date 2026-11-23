CREATE TABLE principals (
    id UUID PRIMARY KEY,
    subject TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status IN ('active', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE api_keys (
    id UUID PRIMARY KEY,
    principal_id UUID NOT NULL REFERENCES principals(id),
    key_hash TEXT NOT NULL UNIQUE,
    scopes JSONB NOT NULL DEFAULT '[]'::jsonb,
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE sources (
    id UUID PRIMARY KEY,
    friendly_name TEXT NOT NULL,
    source_type TEXT NOT NULL,
    desired_state TEXT NOT NULL CHECK (desired_state IN ('disabled', 'enabled')),
    current_revision BIGINT NOT NULL CHECK (current_revision > 0),
    automation_number TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE source_revisions (
    id UUID PRIMARY KEY,
    source_id UUID NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    revision BIGINT NOT NULL CHECK (revision > 0),
    lifecycle TEXT NOT NULL CHECK (lifecycle IN ('draft', 'validated', 'active', 'retired')),
    configuration JSONB NOT NULL,
    secret_references JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source_id, revision),
    UNIQUE (source_id, id, revision)
);

ALTER TABLE sources
    ADD CONSTRAINT sources_current_revision_fk
    FOREIGN KEY (id, current_revision)
    REFERENCES source_revisions (source_id, revision)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE destinations (
    id UUID PRIMARY KEY,
    friendly_name TEXT NOT NULL,
    destination_type TEXT NOT NULL,
    desired_state TEXT NOT NULL CHECK (desired_state IN ('disabled', 'enabled')),
    current_revision BIGINT NOT NULL CHECK (current_revision > 0),
    automation_number TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE destination_revisions (
    id UUID PRIMARY KEY,
    destination_id UUID NOT NULL REFERENCES destinations(id) ON DELETE CASCADE,
    revision BIGINT NOT NULL CHECK (revision > 0),
    lifecycle TEXT NOT NULL CHECK (lifecycle IN ('draft', 'validated', 'active', 'retired')),
    configuration JSONB NOT NULL,
    secret_references JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (destination_id, revision),
    UNIQUE (destination_id, id, revision)
);

ALTER TABLE destinations
    ADD CONSTRAINT destinations_current_revision_fk
    FOREIGN KEY (id, current_revision)
    REFERENCES destination_revisions (destination_id, revision)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE operations (
    id UUID PRIMARY KEY,
    kind TEXT NOT NULL,
    state TEXT NOT NULL CHECK (
        state IN ('accepted', 'running', 'waiting_for_capacity', 'succeeded',
                  'partially_succeeded', 'failed', 'cancelled')
    ),
    cancellable BOOLEAN NOT NULL DEFAULT FALSE,
    progress JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ
);

CREATE TABLE route_instances (
    id UUID PRIMARY KEY,
    source_id UUID NOT NULL REFERENCES sources(id),
    destination_id UUID NOT NULL REFERENCES destinations(id),
    route_revision BIGINT NOT NULL CHECK (route_revision > 0),
    global_revision BIGINT NOT NULL CHECK (global_revision > 0),
    desired_state TEXT NOT NULL CHECK (desired_state IN ('desired', 'removed')),
    operation_id UUID REFERENCES operations(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (destination_id, route_revision)
);

CREATE UNIQUE INDEX route_instances_one_desired_source_per_destination
    ON route_instances (destination_id)
    WHERE desired_state = 'desired';

CREATE INDEX route_instances_source_lookup ON route_instances (source_id);

CREATE TABLE route_events (
    id UUID PRIMARY KEY,
    route_instance_id UUID NOT NULL REFERENCES route_instances(id),
    observed_state TEXT NOT NULL,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE route_presets (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    revision BIGINT NOT NULL CHECK (revision > 0),
    policy JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE preset_routes (
    preset_id UUID NOT NULL REFERENCES route_presets(id) ON DELETE CASCADE,
    destination_id UUID NOT NULL REFERENCES destinations(id),
    source_id UUID NOT NULL REFERENCES sources(id),
    PRIMARY KEY (preset_id, destination_id)
);

CREATE TABLE signal_mappings (
    id UUID PRIMARY KEY,
    source_id UUID NOT NULL REFERENCES sources(id),
    revision BIGINT NOT NULL CHECK (revision > 0),
    match_policy JSONB NOT NULL,
    actions JSONB NOT NULL,
    timing_policy JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL CHECK (status IN ('enabled', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source_id, revision)
);

CREATE TABLE workers (
    id UUID PRIMARY KEY,
    stable_name TEXT NOT NULL UNIQUE,
    platform TEXT NOT NULL,
    architecture TEXT NOT NULL,
    version TEXT NOT NULL,
    state TEXT NOT NULL,
    capabilities JSONB NOT NULL DEFAULT '{}'::jsonb,
    capacity JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_heartbeat TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE pipeline_assignments (
    id UUID PRIMARY KEY,
    resource_type TEXT NOT NULL,
    resource_id UUID NOT NULL,
    worker_id UUID REFERENCES workers(id),
    generation BIGINT NOT NULL CHECK (generation > 0),
    desired_state TEXT NOT NULL,
    observed_state TEXT NOT NULL,
    lease_expires_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (resource_type, resource_id, generation)
);

CREATE INDEX pipeline_assignments_worker_state
    ON pipeline_assignments (worker_id, desired_state, observed_state);
CREATE INDEX pipeline_assignments_expired_lease
    ON pipeline_assignments (lease_expires_at)
    WHERE lease_expires_at IS NOT NULL;

CREATE TABLE operation_items (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operations(id) ON DELETE CASCADE,
    resource_type TEXT NOT NULL,
    resource_id UUID NOT NULL,
    state TEXT NOT NULL,
    result JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX operations_state_created ON operations (state, created_at);

CREATE TABLE audit_records (
    id UUID PRIMARY KEY,
    principal_id UUID REFERENCES principals(id),
    operation_id UUID REFERENCES operations(id),
    action TEXT NOT NULL,
    resource_type TEXT,
    resource_id UUID,
    safe_diff JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX audit_records_resource_time
    ON audit_records (resource_type, resource_id, occurred_at DESC);

CREATE TABLE idempotency_records (
    id UUID PRIMARY KEY,
    scope TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    operation_id UUID REFERENCES operations(id),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (scope, idempotency_key)
);

CREATE INDEX idempotency_records_expiry ON idempotency_records (expires_at);
