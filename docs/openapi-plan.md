# OpenAPI 3.1 Design Plan

**Status:** Phase 1 profile planning and source/destination draft and validation endpoints documented; full contract remains in progress
**Base path:** `/api/v1`
**Wire format:** JSON; OpenAPI 3.1 is the authoritative HTTP contract.

The implemented route planner returns the existing `mediaPath` plus an
`estimate` containing provisional capacity units and nullable payload rates.
This additive response does not check or reserve capacity. The UI tolerates
older API responses without an estimate and reports that it is unavailable.

## Contract-wide behavior

- Plural resource names; stable UUIDs; RFC 3339 UTC timestamps; explicit units
  in field names; machine-stable enums/codes.
- Mutations are REST operations, not WebSocket messages. Return
  `application/problem+json` compatible with RFC 9457, with ARC error code,
  correlation ID, field violations and safe current revision where relevant.
- Revisioned resources return `ETag` and explicit revision. Updates require
  `If-Match`; support `PUT` and JSON Merge Patch. Create/activation semantics
  are explicit. Retryable mutations accept scoped idempotency keys.
- Long operations return `202 Accepted`, `Location: /api/v1/operations/{id}`,
  progress, item results, cancellation eligibility and terminal outcome.
- Use cursor pagination, bounded filtering/sorting/expansion, request-size and
  rate limits. Never reveal secrets; return configured reference and
  `isConfigured`.
- Authenticate REST and WebSocket. WebSocket is read-only, supports snapshot
  bootstrap/reconnect/replay cursors, and delivers a versioned event envelope.

## Resource families

| Family | Representative endpoints / behavior |
|---|---|
| System and capabilities | `GET /system`, `/capabilities`, `/version`; deployment/platform limitations |
| Network interfaces | `GET /network/interfaces`; stable ID, addresses, MTU, link/admin state, multicast, speed |
| Sources | `GET/POST /sources`, `GET/PATCH/PUT/DELETE /sources/{id}`; revisions, validate, probe, apply, enable/start/stop/restart |
| Destinations | Parallel resource lifecycle; codec/capabilities, dry run, apply and runtime actions |
| Routes | `GET /routes`; validate/dry-run, assign/replace/remove, batch/fan-out, take/cancel, rollback, locks and revision conflicts |
| Presets | CRUD/revisions, validate, apply strict/force policy, operation result |
| Signals/GPIO | Mappings and policies; current logical state, recent transitions, GPO state/pulse operations |
| Workers | Registration/heartbeat internal contract; operator-facing workers, capabilities, capacity, assignments and drain operations |
| Operations | `GET /operations/{id}`, list/query, cancellation and per-item outcome |
| Health/statistics | Liveness/readiness, component/endpoint health, bounded runtime statistics |
| Events/bootstrap | `GET /bootstrap`; authenticated read-only `WS /api/v1/events` with subscriptions and replay cursor |
| Audit/auth/keys/secrets | Read/query audit under scope; bootstrap principal, API keys/scopes/revoke, secret references without reveal |
| Backup/restore | Create/export and restore as asynchronous, auditable operations |
| Licensing | Status/capability response and administration where enabled; details remain subject to product decision |

Specific paths are proposals. `openapi/openapi.yaml` is a Phase 0 skeleton, not
the full API contract; expand and validate it before implementing those
operations. Zephyr APIs must support disabled draft, validation,
socket probe, revision apply, lifecycle, detected sender/identities/format,
GPIO mask/pin state, heartbeat/continuity, recent transitions, and sanitized
diagnostics.

## Event envelope

Proposed stable fields: `schemaVersion`, `eventId`, `sequence`, `type`,
`occurredAt`, `resourceType`, `resourceId`, `revision`, `operationId`,
`correlationId`, and a typed `data` payload. Filter subscriptions by bounded
event/resource categories. Define delivery as at-least-once with deduplication
by event ID; replay may be unavailable past configured retention, in which
case the server directs clients to refresh a snapshot.

## Generated clients and docs

Publish interactive API docs; generate a TypeScript SDK from OpenAPI. Keep a
C# SDK generation-compatible contract assumption. CI checks schema lint,
backward-compatibility policy, examples, and API contract tests.
