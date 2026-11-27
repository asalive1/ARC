# Changelog

## Unreleased

### Changed
- Removed the misleading Shift+F10 hint from the crosspoint grid; it opens the context menu rather than taking a route.
- Prevented an unavailable SRT destination from blocking shared-source preview:
  the sink drops media while disconnected and resumes when the peer connects.
- Deployed experimental fenced audio preview to the test host and fixed clean
  WebSocket shutdown. Live ALSA and SRT previews and both RTP outputs carried
  audio; the SRT destination remains unreachable and unverified end-to-end.

### Added
- Added saved mixed Take/Unroute salvos: crosspoints cycle blue, red, clear; red disconnects the selected output regardless of its input, using acknowledged durable Stop execution.
- Added a Linux host-network worker overlay for dual-network media, loopback-only database/WSS control, private-CA trust and non-root ALSA access; deployed stereo ALSA-to-RTP fan-out and sibling-preserving Unroute pass on the test host.
- Added worker WebSocket keepalives to prevent idle TLS-proxy disconnects; all ten control-client tests and the deployed 95-second idle-session check pass.
- Added revision-guarded deletion of disabled, unused inputs and outputs, with confirmation and protection for route history, assignments, signals and saved salvos.
- Reworked input/output configuration into full-width collapsed creation panels and expandable inline editors; enabled resources retain visible, explained editing/deletion restrictions.
- Added saved-salvo management and strict, revision-checked Take execution above the crosspoint grid.
- Added crosspoint Take/Unroute context menus with keyboard access; kept left-click selection as a non-executing preview.
- Added durable, revision-guarded destination Unroute operations with acknowledged Stop, idempotent retry, destination serialization, and explicit unconfirmed-stop outcomes.
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
- Enlarged and repositioned the crosspoint input and output direction labels.
- Simplified crosspoint hover text to the input and output names.
- Prepared the UI proxy to forward WebSocket upgrades for future preview delivery.
- Simplified the sidebar title and removed the requested crosspoint and admin subtitles.
- Prevented shared-source branch changes from racing in-flight GStreamer media
  by holding the tee sink stream lock during branch activation and teardown.
- Began an experimental, bounded worker PCM preview tap for active shared
  sources and a browser PCM player with stereo meters.
- Made worker preview reception asynchronous, reporting source or preview
  failure and discarding queued audio when the last route stops.
- Added a browser preview-stream lifecycle adapter.
- Exposed the existing active-source PCM tap through the worker media
  interface, with an explicit unsupported result for other implementations.
- Added a bounded worker preview relay with disconnect cleanup and an isolated
  RTP audio and route-continuity regression.
- Added a bounded dedicated-WebSocket sender and cancellable browser connection
  setup for preview.
- Added a worker preview connection using its configured control origin,
  credential and TLS trust.
- Connected the in-development authenticated preview relay, input controls,
  crosspoint menu and stereo meters. Preview is not yet deployed.
- Added an experimental reserved-source media tap for future idle-input preview;
  it is not exposed until placement and ownership recovery are implemented.
- Added in-development preview capacity reservations coordinated with route
  placement and registered worker sessions.
- Added worker control support for reserved-source previews and explicit cleanup
  confirmation; the worker exits if reserved-source cleanup cannot be confirmed.
  Idle previews and automatic fencing are not yet enabled.
- Added transactional cleanup for a confirmed-closed preview control session,
  including reservations created before a start command was delivered. Worker
  session finalization remains opt-in pending rollout validation.
- Added an experimental Linux host-process fence primitive. It is not yet
  enabled by default.
- Added transactional storage for process-fence identities and verified stale
  preview recovery. An opt-in Linux worker startup path holds the fence and
  finalizes preview sessions. An experimental opt-in API relay uses fenced
  reservations and confirmed cleanup; it is not yet ready for deployment.
- Added experimental preview decoding of Opus, MPEG-1 audio and AAC (ADTS) in active RTP or
  SRT MPEG-TS sources; unsupported audio codecs are explicitly rejected.
- Show approved resource configurations as Active without an assignment-state
  label, and clarify that Active does not confirm audio flow.
- Clarified SRT caller/listener
  addresses, and safe recovery guidance for an occupied output.
- Replaced the bottom Immediate/Take controls with crosspoint menus and moved compatibility/worker controls into collapsed advanced diagnostics.
- README and roadmap now describe ARC rather than the project template.
- API startup now requires PostgreSQL, applies SQLx migrations, and keeps its non-container bind address loopback-only.

### Fixed
- Failed Takes now clean up their assignment with an acknowledged Stop before
  releasing capacity, so a later Take is not blocked by the abandoned plan.
  Unconfirmed cleanup retains the reservation and reports a recovery failure.
- Reset the Salvo dropdown and draft on navigation without discarding in-flight media operations; late save/delete completions cannot restore an earlier visit's draft.
- Start the Salvo grid blank without live-route overlays; show only draft/saved selections and default the dropdown to "Select Salvo".
- Show the API's explanatory error message and preserve its code/status for rejected UI requests, including unsupported route conversions, instead of displaying only HTTP 422.
- Prevented new Take, salvo execution, and direct route writes from superseding unresolved destination execution or replacement cleanup; accepted idempotent retries remain valid.
- Isolated real destination-element flow errors from shared-source fan-out, preserving sibling delivery without restarting the source and reporting failures on the affected route.
- Kept far-corner crosspoints reachable when endpoint catalogs load after the matrix first renders, and prevented crosspoint tooltips from blocking neighboring cells.
- Prevented SRT output sinks from blocking live pipeline startup while waiting for preroll; clock synchronization remains enabled.
