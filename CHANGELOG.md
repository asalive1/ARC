# Changelog

## Unreleased

### Added
- Approved ARC architecture package and Phase 0 Rust/API/worker/media contract scaffolding.
- PostgreSQL initial migration, OpenAPI skeleton, Zephyr fixture area, and React health UI.
- Local Docker Compose stack for the API, UI, and PostgreSQL, plus a source dependency license inventory.
- Versioned WebSocket/JSON selected for API-to-worker control; credential provisioning remains open.
- Phase 1 media profile selected: RTP MP2T to SRT MPEG-TS pass-through; LGPL GStreamer and MPL-2.0 SRT approved, with GPL/AGPL excluded.
- Added RTP L24 input/output profiles and matching RTP L24 route planning without L16/L24 conversion.
- Added a technology licensing guide and a read-only, heartbeat-aware worker placement decision.
- Added PostgreSQL worker registry persistence with database-timestamped heartbeats and drain-state protection; worker transport remains planned.
- Added PostgreSQL-backed atomic capacity reservations; worker dispatch and media execution remain planned.
- Added generation-numbered planned assignments with reservation protection and safe cancellation of unstarted plans.
- Added persistent worker drain requests that prevent new reservations and plans without discarding existing commitments.
- Added provider-backed per-worker bearer credential verification; deployment-specific secret stores and credential lifecycle remain open.
- Added an injectable TLS-gated worker WebSocket hello/welcome session; command and event handling remains disabled.
- Added a local-only JSON environment secret provider and conditional API startup wiring for the worker route.
- Added a reservation-aware, advisory PostgreSQL implementation of the worker scheduler interface.
- Added provisional, deterministic requirement estimates for supported Phase 1 media paths.
- Exposed provisional route estimates in the planning API and compatibility view without reserving capacity.
- Added an internal transaction that reserves capacity and creates a planned assignment together.
- Persisted endpoint revision references and immutable route configuration snapshots with planned assignments.
- Exposed revision-pinned, capacity-reserving planning through `POST /api/v1/routes/plan-assignment`; retained `/routes/plan` as a non-reserving compatibility check.
- Added assignment lookup by pipeline and generation-fenced cancellation of unstarted plans; cancellation releases its reservation atomically.
- Added worker/pipeline identities to command envelopes and a transport-independent worker command gate for generation fencing and completed-command deduplication.
- Added PostgreSQL-backed worker assignment fences and a durable command ledger, with integration coverage for persistence, duplicate completion, and concurrent generation updates.
- Connected the authenticated worker WebSocket session to a bounded command dispatcher with per-pipeline serialization, session replacement, and completion-acknowledged durable command handling.
- Added a worker-side transport-independent command receiver that gates generations before handler invocation and returns completed or stale acknowledgements.
- Added a worker WebSocket client with bounded messages, TLS-by-default endpoint validation, welcome verification, and generation-gated Start/Stop handlers using the injected media pipeline interface.
- Reset worker reconnect backoff after a successful welcome and preserve the established-session marker across the session status transition.
- Added API-dispatched live RTP MPEG-TS-to-SRT GStreamer loopback coverage and forwarded-protocol headers for loopback worker control tests.
- Added worker startup reconciliation for persisted starting/running and stopping assignments.
- Added PostgreSQL-backed latest-session leases for cross-process worker replacement fencing.
- Added database-backed API readiness and Compose API health checks.
- Made unstarted assignment cancellation repeatable for the same exact generation while rejecting active or uncertain lifecycle states.
- Added worker heartbeat ingress/egress limits and CPU/memory pressure admission guards.
- Documented explicit ALSA device exposure for the opt-in worker container.
- Added file-backed Docker secret startup inputs and a WSS Nginx ingress Compose override.
- Added isolated PostgreSQL clean-migration, restart, dump, and restore acceptance evidence script.
- Added the `arc-gstreamer` adapter for supported Phase 1 paths, a worker executable with PostgreSQL registration/heartbeat and control reconnect, and an opt-in Compose worker profile.
- Added a live GStreamer RTP MPEG-TS-to-SRT loopback test that verifies the original transport-stream payload reaches an SRT listener.
- Added a generation-checked Start lifecycle endpoint that dispatches the persisted route snapshot to the assigned worker and records running state only after acknowledgement.
- Added a generation-checked Stop lifecycle endpoint that retains capacity until worker acknowledgement, then atomically marks the assignment stopped and releases its reservation.
- Added assignment-managed source/destination enablement: Start enables route resources, while acknowledged Stop or cancellation disables resources with no other active assignment.
- Added operator UI controls to reserve a worker for a validated single-route assignment, inspect its lifecycle, and issue generation-fenced Start/Stop or cancel an unstarted assignment.
- Added PostgreSQL-backed 24-hour replay for planning requests using a required Idempotency-Key, with key/body mismatch conflicts.

### Changed
- README and roadmap now describe ARC rather than the project template.
- API startup now requires PostgreSQL, applies SQLx migrations, and keeps its non-container bind address loopback-only.

### Fixed
- Prevented SRT output sinks from blocking live pipeline startup while waiting for preroll; clock synchronization remains enabled.
