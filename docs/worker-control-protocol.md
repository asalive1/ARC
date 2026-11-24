# Worker Control Protocol

**Status:** The provider-backed credential verifier, local-only JSON environment provider, conditional API executable wiring, TLS-gated WebSocket session, PostgreSQL-backed command dispatcher, worker WebSocket client, generation-gated Start/Stop handlers, GStreamer media adapter, and opt-in worker Compose profile are implemented. The UI supports a validated single-route assignment lifecycle; multi-route/resource lifecycle UI and production secret stores remain incomplete.

## Decision

API-to-worker control and worker observations use a full-duplex WebSocket
connection carrying versioned JSON messages. Each connection is authenticated
as a worker service identity and bound to that worker's registered `workerId`.
The selected credential strategy is per-worker bearer credentials resolved
through the existing `SecretProvider` abstraction. A deployment must use TLS
for this connection. Concrete provider adapters, provisioning and rotation
remain subject to [Q13](open-questions.md).

The infrastructure authenticator resolves each credential from
`workers/{workerId}/control-token`, compares bearer values in constant time,
and returns generic rejection details. Provider failures are logged without
credential contents and reported as an authentication-service failure.

For a local POC, `LocalWorkerSecretProvider` loads
`ARC_WORKER_CREDENTIALS_JSON` at API startup. The value is a JSON object mapping
canonical worker UUIDs to 64-character hexadecimal bearer tokens. Generate
tokens with a cryptographically secure random source, keep the value out of
source control and logs, and restart the API to apply credential changes. This
environment-backed adapter is not a production secret store.

Workers initiate WSS to `GET /api/v1/workers/control`. The upgrade request
carries `Authorization: Bearer <token>` and `X-Worker-ID`. TLS terminates at a
trusted reverse proxy or ingress, which must overwrite `X-Forwarded-Proto` with
`https`; the API must only be reachable from that trusted network. The API
rejects requests without that marker as defense in depth. Do not log the
Authorization header.

After upgrade, the worker sends a JSON hello such as
`{"kind":"hello","protocolVersion":1,"workerId":"<worker-uuid>"}` within five
seconds. The API verifies that the version and identity match the authenticated
request, then replies with a `welcome` message containing the protocol version
and worker ID. The current message and frame limit is 64 KiB. This limit and
the five-second hello timeout are initial implementation defaults, not a
final transport capacity decision.

When `ARC_WORKER_CREDENTIALS_JSON` is nonempty and valid, the API executable
composes the local provider with the authenticated route. When it is absent or
empty, the worker route is not registered. Compose accepts this setting for the
API and provides an opt-in worker profile, but requires an independently
configured trusted WSS ingress; it does not add a TLS proxy or development CA.

## Versioned envelopes

The shared Rust crate defines the protocol-v1 envelopes:

- `WorkerCommandEnvelope<T>` carries `protocolVersion`, `commandId`,
  authenticated `workerId`, `pipelineId`, `assignmentId`, `generation`, and a
  typed payload from API to worker.
- `WorkerEventEnvelope<T>` carries `protocolVersion`, `eventId`, `workerId`,
  optional `assignmentId` and `generation`, and a typed payload from worker to
  API.

Fields use camelCase JSON names. Every message must include the supported
protocol version. A peer must explicitly reject an unsupported version; it
must not silently interpret or downgrade it.

The worker crate provides a transport-independent command gate. It validates
the protocol version, nonnil identities, positive generation and equality
between the envelope `workerId` and the authenticated session identity. It
then uses an `AssignmentGenerationStore` to atomically fence older generations
and completed command IDs before calling a command handler. Incomplete
commands can be retried after restart; handlers must therefore be idempotent
by `commandId`. `PgAssignmentGenerationStore` persists the observed assignment
generation and command ledger in PostgreSQL (migration 0005), with transactional
generation updates and completed-command deduplication tested across store
instances. The API dispatcher validates/fences a command before sending it,
serializes dispatches for a pipeline within one API process, and marks a command
complete only after a correlated `command_ack` with outcome `completed`. The
worker crate's receiver applies the same generation gate before invoking its
injected handler. It returns `completed` only after the handler succeeds and the
durable store marks the command complete; an already completed duplicate also
returns `completed` without invoking the handler. A stale command returns
`stale` with `currentGeneration` and never reaches the handler. The API treats
that as a dispatch error and does not mark the command complete. An
unacknowledged command remains retryable; a generation first becomes known when
a command reaches the store, so the store alone cannot reject an older command
before a newer generation has been observed. Session replacement and
per-pipeline serialization are process-local, and ledger retention is not
bounded.

The acknowledgement message uses the normal camelCase envelope fields:
`kind`, `protocolVersion`, `workerId`, `commandId`, and `outcome`. A stale
acknowledgement also includes `currentGeneration`. The dispatcher accepts
`kind: "command_ack"` with `outcome: "completed"` or `"stale"`. The worker
client sends the authenticated worker ID and bearer credential, verifies the
welcome identity and protocol version, and bounds messages and frames to 64 KiB.
Plaintext `ws://` is rejected except when explicitly enabled for an IP loopback
endpoint. Its typed command payloads currently support `start` with a
`PipelineSpecification` and `stop`; Start validates the pipeline ID and
generation, then calls the injected `MediaPipeline`'s `prepare` and `start`
methods, while Stop calls `stop`. A successful acknowledgement is emitted only
after the injected operation and durable completion both succeed. The worker
executable injects the `arc-gstreamer` adapter, which converts typed route
snapshots to supported GStreamer graphs, observes pipeline state and bus errors,
and advertises only installed plugin capabilities. The worker registers and
heartbeats through PostgreSQL and reconnects its authenticated control session
with bounded backoff. `POST /api/v1/routes/plan-assignment` continues to create
a plan without executing it. `POST /api/v1/assignments/{assignmentId}/start`
is the explicit lifecycle action: it verifies the generation, persists the
starting state, and dispatches a Start command containing the assignment's
revision-pinned route snapshot. The assignment becomes running only after the
worker acknowledges completion. Repeating Start for a running assignment is
idempotent; an unavailable worker leaves the plan retryable.
Initial dispatcher limits are 16 queued commands per session and a 30-second
completion-acknowledgement timeout. Queue saturation, session loss, or timeout
returns an error and leaves the command retryable.

## Registry and placement foundations

The PostgreSQL registry adapter implements the internal `WorkerRegistry`
port. It stores unique worker IDs and stable names, requires platform and
architecture, and timestamps registration and heartbeat receipt using the
database clock rather than the reported worker timestamp. New registrations
start in `registering` state with no assignments; duplicate identities are
rejected. Heartbeats require the registered ID, stable name, platform and
architecture, and cannot clear controller-owned drain states. A new registry
instance can read the stored worker records.

The worker executable now registers and heartbeats through this adapter and
reports `ready` only while its authenticated control session is connected.
The adapter still does not authenticate database callers, complete drain, or
reconcile assignments. Heartbeat capacity is a reported snapshot, not an
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

The planner persists exact source and destination revision references with the
media path and configuration snapshot on the assignment. Revision foreign keys
keep the referenced history intact, and row locks plus expected-current-
revision checks reject plans racing with endpoint edits. Only validated or
active revisions may be planned.

`POST /api/v1/routes/plan-assignment` exposes this planning operation
separately from `POST /api/v1/routes/plan`, which remains a non-reserving
compatibility and estimate check. The request names a client-generated
pipeline ID and expected source and destination revisions. Heartbeat freshness
is fixed by the server to a 30-second maximum; clients cannot relax it. The
response reports `planned` with the assignment and pinned revisions, or
`unavailable` with ordered, per-worker rejection reasons and no assignment or
reservation. `GET /api/v1/pipelines/{pipelineId}/assignment` returns the
latest generation and its desired/observed states, including cancelled
assignments. Clients can use this to reconcile an uncertain planning or
cancellation commit rather than assuming a connection failure means rollback.
`POST /api/v1/assignments/{assignmentId}/cancel` requires the exact generation
and atomically cancels only a `planned`, `not_started` assignment while
releasing its reservation. It cannot stop a running pipeline. Planning requests
require a non-nil UUID `Idempotency-Key`. The same key and request body replay
the original planned or unavailable result for 24 hours; reusing a key with a
different body returns a conflict. The key is distinct from the stable pipeline
ID, so a new key continues through the existing assignment-generation and
duplicate-pipeline rules.

`POST /api/v1/assignments/{assignmentId}/stop` requires the exact generation
and only accepts assignments that have entered the running lifecycle.
The generation-checked Start and Stop endpoints dispatch commands for planned
assignments. Start uses the persisted route snapshot and records `running` only
after worker acknowledgement. Stop first records desired `stopped` and observed
`stopping`; it retains the reservation until acknowledgement, then atomically
records `stopped` and releases capacity. An unavailable worker, timeout, or
stale acknowledgement leaves the assignment retryable in `stopping` with its
reservation held. Retrying Stop uses a stable command identity, and an
already-stopped assignment is returned without another dispatch.

The operator UI can create and control a single validated route assignment.
Starting an assignment enables its pinned source and destination; they remain
enabled while the worker is starting, running, or stopping. After acknowledged
Stop, each resource is disabled only when no other active assignment uses it.
Planning alone does not start media, and workers must independently fence
generations before executing commands.

The WebSocket session sends versioned command envelopes and receives correlated
completion acknowledgements; it still rejects binary frames and does not
process worker events. The worker executable runs the WebSocket client and
registers Start/Stop media handlers. Cross-process session leases and restart
reconciliation remain unimplemented. Workers independently fence generations
before executing commands; API-side dispatch alone does not make stale
in-flight work safe.

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

Phase 1 still needs to define cross-process heartbeat/lease behavior,
connection limits, production secret-provider adapters, command/event payload
schemas, and restart reconciliation.
