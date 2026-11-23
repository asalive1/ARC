# PostgreSQL Database ERD

**Status:** Approved architecture baseline; initial migration and SQLx runner
validated; repository and upgrade/recovery coverage remains planned
**Database:** PostgreSQL only; SQLx migrations are the schema-versioning
mechanism. SQLite and a multi-database abstraction are out of scope.

```mermaid
erDiagram
    PRINCIPAL ||--o{ API_KEY : owns
    SOURCE ||--o{ SOURCE_REVISION : versions
    DESTINATION ||--o{ DESTINATION_REVISION : versions
    SOURCE ||--o{ ROUTE_INSTANCE : feeds
    DESTINATION ||--o{ ROUTE_INSTANCE : receives
    ROUTE_INSTANCE ||--o{ ROUTE_EVENT : records
    SOURCE ||--o{ SIGNAL_MAPPING : triggers
    WORKER ||--o{ PIPELINE_ASSIGNMENT : owns
    PIPELINE_ASSIGNMENT ||--o{ OPERATION : advances
    OPERATION ||--o{ OPERATION_ITEM : contains
    PRINCIPAL ||--o{ AUDIT_RECORD : acts
    OPERATION ||--o{ AUDIT_RECORD : correlates
    ROUTE_PRESET ||--o{ PRESET_ROUTE : contains
    DESTINATION ||--o{ PRESET_ROUTE : targets
    SOURCE ||--o{ PRESET_ROUTE : selects
    IDEMPOTENCY_RECORD }o--|| OPERATION : resolves_to

    PRINCIPAL {
      uuid id PK
      text subject UK
      text status
      timestamptz created_at
    }
    API_KEY {
      uuid id PK
      uuid principal_id FK
      text key_hash UK
      jsonb scopes
      timestamptz expires_at
      timestamptz revoked_at
    }
    SOURCE {
      uuid id PK
      text friendly_name
      text source_type
      text desired_state
      bigint current_revision
      text automation_number
    }
    SOURCE_REVISION {
      uuid id PK
      uuid source_id FK
      bigint revision
      text lifecycle
      jsonb config
      text secret_refs
      timestamptz created_at
    }
    DESTINATION {
      uuid id PK
      text friendly_name
      text destination_type
      text desired_state
      bigint current_revision
      text automation_number
    }
    DESTINATION_REVISION {
      uuid id PK
      uuid destination_id FK
      bigint revision
      text lifecycle
      jsonb config
      text secret_refs
      timestamptz created_at
    }
    ROUTE_INSTANCE {
      uuid id PK
      uuid source_id FK
      uuid destination_id FK
      bigint route_revision
      bigint global_revision
      text desired_state
      uuid operation_id
      timestamptz created_at
    }
    ROUTE_EVENT {
      uuid id PK
      uuid route_instance_id FK
      text observed_state
      jsonb details
      timestamptz occurred_at
    }
    ROUTE_PRESET {
      uuid id PK
      text name UK
      bigint revision
      jsonb policy
    }
    PRESET_ROUTE {
      uuid preset_id FK
      uuid destination_id FK
      uuid source_id FK
    }
    SIGNAL_MAPPING {
      uuid id PK
      uuid source_id FK
      bigint revision
      jsonb match_policy
      jsonb actions
      text timing_policy
      text status
    }
    WORKER {
      uuid id PK
      text stable_name UK
      text platform
      text architecture
      text version
      text desired_state
      timestamptz last_heartbeat
    }
    PIPELINE_ASSIGNMENT {
      uuid id PK
      text resource_type
      uuid resource_id
      uuid worker_id FK
      bigint generation
      text desired_state
      text observed_state
      timestamptz lease_expires_at
    }
    OPERATION {
      uuid id PK
      text kind
      text state
      boolean cancellable
      jsonb progress
      timestamptz created_at
      timestamptz completed_at
    }
    OPERATION_ITEM {
      uuid id PK
      uuid operation_id FK
      text resource_type
      uuid resource_id
      text state
      jsonb result
    }
    AUDIT_RECORD {
      uuid id PK
      uuid principal_id FK
      uuid operation_id FK
      text action
      text resource_type
      uuid resource_id
      jsonb safe_diff
      timestamptz occurred_at
    }
    IDEMPOTENCY_RECORD {
      uuid id PK
      text scope
      text key
      text request_hash
      uuid operation_id FK
      timestamptz expires_at
    }
```

## Modeling decisions

- Source/destination identity is stable; immutable revisions hold draft,
  validated, and active configurations. Never store secret values in `jsonb`;
  persist secret references only.
- Interface selection is a stable host-interface identifier in the relevant
  revision config. Live inventory is observed worker state and may change; API
  should expose it without treating it as durable configuration.
- Desired routes are unique by destination (at most one source per
  destination). Route instances/history are immutable and revisions are
  monotonic. Encode the one-source constraint with a unique active desired
  destination key or equivalent transactional constraint.
- Assignments carry desired/observed states, worker lease and monotonically
  increasing generation. Worker heartbeat/capacity telemetry is not an
  unbounded durable time series; retain only operationally useful snapshots.
- Operations are durable for recovery and clients; operation items record
  independent batch outcomes. Idempotency keys are scoped and expire by
  operation class. Audit payloads are redacted and append-only by application
  policy.
- Event sequence/replay metadata, auth users/roles, secrets, device connections,
  backups, licensing, locks, and configuration import/restore may require
  additional normalized tables based on reviewed retention and ownership
  decisions; see [open questions](open-questions.md).

The Phase 0 migration scaffold is in `database/migrations/0001_initial.sql`;
it has been applied to PostgreSQL 16, including current-revision foreign keys
and route cardinality validation; the SQLx migrator test also passes against a
disposable PostgreSQL 16 database. Upgrade/recovery paths still require
integration tests. Migrations must add indexes for foreign keys, resource/revision lookups,
operation state, lease expiry, and idempotency uniqueness; use transactions and
test both clean installation and upgrade paths. JSONB is for adapter-specific
configuration, not core relational invariants.
