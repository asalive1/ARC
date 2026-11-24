# Worker Control Protocol

**Status:** Transport decision approved; runtime contract is not yet implemented.

## Decision

API-to-worker control and worker observations use a full-duplex WebSocket
connection carrying versioned JSON messages. Each connection is authenticated
as a worker service identity and bound to that worker's registered `workerId`.
Credential provisioning and rotation remain subject to
[Q13](open-questions.md).

## Versioned envelopes

The shared Rust crate defines the protocol-v1 envelopes:

- `WorkerCommandEnvelope<T>` carries `protocolVersion`, `commandId`,
  `assignmentId`, `generation`, and a typed payload from API to worker.
- `WorkerEventEnvelope<T>` carries `protocolVersion`, `eventId`, `workerId`,
  optional `assignmentId` and `generation`, and a typed payload from worker to
  API.

Fields use camelCase JSON names. Every message must include the supported
protocol version. A peer must explicitly reject an unsupported version; it
must not silently interpret or downgrade it.

## Registry and placement foundations

The PostgreSQL registry adapter implements the internal `WorkerRegistry`
port. It stores unique worker IDs and stable names, requires platform and
architecture, and timestamps registration and heartbeat receipt using the
database clock rather than the reported worker timestamp. New registrations
start in `registering` state with no assignments; duplicate identities are
rejected. Heartbeats require the registered ID, stable name, platform and
architecture, and cannot clear controller-owned drain states. A new registry
instance can read the stored worker records.

The adapter is not connected to a worker service or network endpoint. It does
not authenticate callers, implement reconnect/session fencing, complete drain,
or reconcile assignments. Heartbeat capacity is a reported snapshot, not an
authoritative assignment ledger.

The domain exposes a deterministic, read-only worker selection function. It
selects the lowest worker UUID among workers with a fresh heartbeat, an
accepting state, matching capabilities, and sufficient reported capacity.
If none qualify, it returns ordered per-worker rejection reasons; an empty
inventory returns an unavailable decision with no rejections. Callers supply
the current time and a nonnegative maximum heartbeat age. A heartbeat exactly
at the age limit is valid; future timestamps are rejected. The eventual
registry uses trusted database receipt times for heartbeats.

This function does not register workers, reserve capacity, create assignments,
or start media. Its result is only a snapshot.

The internal PostgreSQL `WorkerScheduler` adapter evaluates a repeatable-read
snapshot of registered workers and active reservations using the same
conservative capacity accounting as admission. It requires positive capacity
and a nonnegative heartbeat-age limit. The detailed inspection method returns
the placement decision with per-worker rejection reasons; the existing
`WorkerScheduler` interface reports an unavailable decision as a
`placement_unavailable` error.

The scheduler is advisory: another transaction can change a worker or consume
capacity immediately after the snapshot. Its result does not reserve anything
and must not be used to start a pipeline. The reservation and assignment
transactions must still perform their own eligibility and capacity checks.
The adapter is not yet connected to a route lifecycle or worker runtime.

An internal domain estimator derives adapter and provisional capacity
requirements from a validated Phase 1 media path. Its assumptions and
unmodeled bandwidth and resource costs are documented in the
[media pipeline design](media-pipeline-design.md). The scheduler can evaluate
those requirements, but neither component activates routes.

## Capacity reservation foundation

The internal PostgreSQL reservation adapter locks worker rows before checking
freshness, capabilities and capacity, and persists the reservation in the
same transaction. Admission subtracts both reported assigned capacity and all
active reservations, conservatively counting both even if they overlap. A
partial unique index permits at most one active reservation per pipeline.
Reservations remain active across reconnects and can be listed or explicitly
released; heartbeats cannot erase them. An updated capacity report can still
show that existing commitments exceed the worker's reduced capacity, in which
case further admission is rejected.

Reservations are not evidence that media is running. They are visible to the
advisory scheduler, but not yet connected to the route lifecycle, lease expiry
or stop acknowledgement.

## Planned assignment foundation

The internal PostgreSQL assignment adapter creates a `planned`, `not_started`
record from an active reservation. Under the worker lock it rechecks the
heartbeat, state, capabilities and all capacity commitments before assigning
a new, monotonically increasing generation for the pipeline. Requirements
and reservation identity are persisted with the assignment. A reservation
linked to any assignment cannot be released directly; cancelling an
unstarted assignment with its exact ID and generation releases it atomically.
The adapter lists planned assignments after reconnect. A stale cancellation,
duplicate active assignment, or cancellation of a started assignment fails.

The internal pipeline planner validates the supplied source and destination,
derives their provisional requirements, and performs reservation and
assignment planning in one PostgreSQL transaction. If no worker is eligible,
it returns the placement rejections without creating a reservation. If
assignment planning fails after reservation, the transaction rolls back both.
Concurrent planners use the same worker-row locking as admission; cancellation
of an unstarted plan releases its reservation.

The planner does not persist source/destination profiles or route revisions,
and is not exposed as a route activation API. A database connection failure
during commit can leave the outcome unknown; callers must inspect persisted
state before retrying rather than assuming failure means rollback.

This is control-plane preparation only: no worker commands are dispatched,
and no running pipeline is implied. Worker-side generation enforcement,
authenticated sessions, dispatch state, stop acknowledgement, leases and
restart reconciliation are not implemented. Before sending commands, the
runtime must make dispatch and cancellation mutually safe, and before
releasing running capacity it must obtain stop acknowledgement or establish
effective fencing.

## Drain request foundation

The internal registry can persist a drain request for a ready or accepting
worker. Requests for an already draining or drained worker are idempotent;
other states are rejected. The request locks the worker row shared by capacity
admission and assignment planning, so subsequent reservations and plans
cannot select that worker. Existing reservations and assignments remain
intact, and heartbeats cannot clear the drain state.

This only records drain intent. It does not notify the worker, stop or move
pipelines, mark the worker drained, or establish that shutdown is safe.
Authenticated observations and stop acknowledgement or effective fencing
are required before a future implementation can make that determination.

## Not frozen by this decision

Phase 1 will define the WebSocket endpoint, connection initiator, handshake
payload, credential mechanism, heartbeat/lease behavior, reconnect policy,
message-size limits, and concrete command/event payload schemas. The Phase 0
API does not expose a worker WebSocket endpoint, and the worker crate is not
yet an executable service.
