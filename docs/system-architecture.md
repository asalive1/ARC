# ARC System Architecture

**Status:** Approved baseline; Phase 0 implementation in progress
**Date:** 2026-11-22
**Scope:** Production-oriented POC architecture; no production code is authorized
by this document.

## 1. Executive decision

Build ARC as a stateful, API-first platform with four logical software layers
and two separated runtime planes. PostgreSQL is the only durable database.
The API/control service owns configuration and orchestration; independently
managed worker processes own media pipelines and host-resource access. Even a
single-machine POC must use the worker, scheduler, assignment, generation, and
reconciliation contracts so additional workers can be introduced without
changing domain behavior.

The Device Server integration is a separate deployable plugin communicating
through a versioned, authenticated WebSocket/JSON protocol. It has no direct
database, ARC-code, or media-pipeline access.

## 2. Governing material and conflict handling

Apply the authority order in Master Prompt V6: V6, approved Jira decisions,
the combined supplemental specification, observed Zephyr protocol analysis,
then only non-superseded older material. V6 supersedes earlier master prompts.
The specifically invoked [ARC master prompt](../Reference%20Docs/GitHub%20Copilot%20Master%20Prompt%20-%20ARC%20%28Audio%20Routing%20Controller%29.md)
also requires approval before implementation; this package honors that gate.

The invoked prompt's Phase 1 demo (ALSA/RTP input to RTP/SRT output) differs
from V6's Phase 1 (generic RTP source, RTP and null/test destinations), with
Zephyr moved to Phase 2 in V6. Use V6 as the proposed technical baseline and
resolve the acceptance-order decision before Phase 1 is committed. Do not treat
draft protocol observations as guarantees where V6 marks values configurable
or unknown.

## 3. Goals and boundaries

### POC goals

- Configure sources, destinations, routes, signals, and workers through common
  domain/application services exposed by REST and browser UI.
- Receive and route real media; isolate failures per pipeline.
- Preserve desired state and reconstruct enabled pipelines after restart.
- Demonstrate RTP and Zephyr Connect audio/GPIO, routing, worker scheduling,
  health/events, and a real network output.
- Deploy the API, worker, UI, PostgreSQL, and OpenTelemetry collector with
  Docker on Linux AMD64 and ARM64 where dependencies permit.
- Build shared Rust control-plane code on Windows x86-64, documenting
  platform-specific media and networking limitations.

### Not a POC goal

Creating or scaling cloud resources, multi-AZ availability, commercial capacity
planning, cluster-wide database failover, scheduled route execution, or
unapproved Zephyr codec profiles. Interfaces must leave room for these without
putting AWS SDK calls or deployment policy in domain/application code.

## 4. Layer and runtime model

| Layer | Responsibilities | Must not own |
|---|---|---|
| Domain | Source/destination/route identities and revisions; invariants; route and signal policies; worker/assignment state; domain events | Axum, SQLx, GStreamer, OS APIs, AWS |
| Application | Use cases; authorization checks; validation; dry runs; idempotency; operations; scheduling; reconciliation; audit/event publication | Direct database or media-framework calls |
| Infrastructure | PostgreSQL/SQLx repositories; GStreamer and OS adapters; network discovery; secrets; event transport; telemetry; local scheduler/capacity manager | Product policy duplicated outside application/domain |
| Presentation | Axum REST/WebSocket, generated OpenAPI; React/TypeScript/Vite/MUI UI | Direct repository, scheduler, or pipeline control |

**Control plane:** API, authentication/authorization, configuration revisions,
desired routes, persistence, operations, scheduler, reconciliation, audit,
licensing, health, events, and observability.

**Media plane:** source receive/demultiplex, decode/pass-through, normalization,
fan-out, encode/mux/transmit, signal extraction/generation, and GPIO transition
detection. Workers own media sockets and discover host network interfaces.
Control-to-worker commands and events use authenticated, versioned WebSocket
JSON as defined in the [worker protocol](worker-control-protocol.md); API
handlers never run media processing inline. The browser event WebSocket remains
read-only and separate from worker control.

The V6 initial POC adapter inventory is Zephyr Connect RTP/UDP, generic RTP,
SRT/MPEG-TS, UDP/MPEG-TS, HLS, Icecast AAC/MP3/Opus, and ALSA or Windows audio
input where available; destinations are RTP, SRT/MPEG-TS, UDP/MPEG-TS, ALSA,
Windows WASAPI shared output, and null/test, with HLS/Icecast publishing
capability reported where implemented. Availability is per-worker capability,
not a promise that every adapter exists on every OS/architecture. Each
destination owns explicit output format, pass-through/encode policy, gain/mute,
channel and sample-rate conversion, delay, no-source behavior, source-failure
policy, retry/recovery, and cue/metadata policy.

## 5. Runtime topology and deployment

The target POC topology is `arc-ui`, `arc-api`, `arc-worker`, `postgres`, and
`otel-collector`. API and worker may share a host, but remain separate logical
services. `arc-worker` needs explicit access to host interfaces, multicast
membership and statistics; Linux Docker host networking is an option where
required, not an implicit assumption. Document port exposure and platform
limitations. PostgreSQL stores desired state and operational records, not
audio samples, jitter buffers, decoder/encoder frames, socket buffers, or
high-rate meter samples.

The Phase 0 Compose stack currently runs only `arc-ui`, `arc-api`, and
`postgres`. Worker and collector services are deferred until their executable
runtime and telemetry export are implemented.

The Device Server plugin is built and deployed independently on supported
Linux/Windows hosts. A plugin endpoint is configured in ARC and communicates
over authenticated, version-negotiated WebSocket JSON. The plugin can reconnect,
publish capabilities and state, accept commands, and reconcile snapshots.

## 6. State and recovery

- **Durable desired state:** source/destination active revisions, desired
  routes, route instances/history, presets, mappings, interface selections,
  automation numbers, worker registrations/assignments/generations, operation
  records, idempotency results, audit records, auth/key metadata, and event
  replay checkpoints where required.
- Source and destination `desired_state` follows assignment lifecycle: Start
  enables the pinned resources before dispatch, and acknowledged Stop disables
  each resource only after its last active assignment releases it.
- **Reconstructable observed state:** worker/pipeline health, capabilities,
  active media flow, interface inventory, and live statistics. Refresh from
  workers/devices; never mistake an observation for user intent.
- **Ephemeral media state:** packet/frame buffers, decoder state, meters, and
  other high-frequency media internals.

On restart: migrate/check schema; register workers; load enabled active
revisions and desired routes; validate against current capabilities and
interfaces; schedule and reconcile; rebuild pipelines; publish recovery
transitions. Unavailable or incompatible desired routes stay visible as failed
or waiting, not silently deleted. Zephyr GPIO establishes a fresh baseline
after restart/reconnect; do not emit a false transition or replay expired cues.

## 7. Routing and isolation invariants

- A destination has zero or one desired source; a source can feed multiple
  destinations subject to admission/capacity limits.
- Mixing and independent left/right routing are not POC functions; a route
  requires compatible source/destination capabilities.
- Desired and observed routing are separate. Route mutations advance server
  revisions and create immutable route instances/operation history.
- Route replacement is an explicit operation. Prepare/validate, check
  permissions/locks/revisions, apply, observe, then commit the resulting state;
  document that cross-worker/media actions cannot be made database-atomic.
- Support immediate and take workflows, atomic and best-effort batches,
  presets/salvos, undo/rollback, locks/permissions, a virtualized 64x64 matrix,
  and real-time multi-user synchronization as phased routing capabilities.
- Fan-out is best-effort by default; a slow or failed destination cannot block
  the source or sibling destinations. Batch operations expose per-item results
  and support explicit rollback where possible.
- Pipeline failures are isolated; a pipeline restart must not restart ARC or
  unrelated pipelines.

## 8. Technology and portability

Rust, Tokio, Axum, Serde, SQLx, PostgreSQL, tracing/OpenTelemetry, and
OpenAPI 3.1; React, TypeScript, Vite, Material UI, TanStack Query, and a
virtualized 64x64 crosspoint grid; GStreamer behind Rust media interfaces.
Required interfaces include `SourceAdapter`, `DestinationAdapter`,
`MediaPipeline`, `SignalExtractor`, `SignalGenerator`,
`NetworkInterfaceProvider`, `StateRepository`, `SecretProvider`,
`EventPublisher`, `WorkerRegistry`, `WorkerScheduler`, and `BackupProvider`.
No business logic imports GStreamer types.

Declare worker capabilities by actual platform, CPU architecture, adapter,
codec, encoder, signal parser/generator, audio backend, and hardware
acceleration. Placement rejects incompatible workers with structured reasons.
Core domain/application modules have no AWS SDK dependency.

## 9. Observability and real-time events

Provide structured logs, traces, metrics, health/readiness endpoints, and audit
events from the first implementation phase. Use a read-only WebSocket event
channel with versioned envelopes, sequence/replay cursors, and snapshot
bootstrap. Include route, GPIO/signal, health, system, worker/assignment, and
operation events. Avoid secrets and high-cardinality identifiers in metric
labels. Expose stable semantics for worker capacity, queue pressure, unplaced
pipelines, assignment counts, draining, and heartbeat age.

## 10. Implementation status

The architecture package in [the index](ARCHITECTURE.md) was approved before
implementation began. The Rust workspace, API, PostgreSQL schema, OpenAPI
contract, media/worker ports, synthetic fixture area, and React UI are in
place. Phase 1 now includes profile planning, durable assignments and fences,
API worker command dispatch, a worker executable, and a typed GStreamer
adapter. The default Compose stack runs the API, UI, and PostgreSQL; an
opt-in worker profile adds the worker. A generation-checked assignment Start action dispatches its persisted route
snapshot and records running state after acknowledgement. Assignment Stop
dispatches a fenced command, retains capacity until worker acknowledgement, and
then releases the reservation atomically and disables resources not used by
another active assignment. The operator UI can reserve and control a validated
single-route assignment. Multi-route matrix editing, telemetry export,
reconciliation, broader endpoint/routing administration, and production
authentication remain unimplemented; phase exit requires the validation and
blocking decisions in the delivery plan and open-questions register.
