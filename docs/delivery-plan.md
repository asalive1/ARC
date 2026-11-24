# Delivery Plan and Git Strategy

**Status:** Phase 1 in progress; Phase 0 exit evidence and remaining Phase 1 items are tracked below.

## Phase gates

| Phase | Deliverable scope | Exit evidence |
|---|---|---|
| 0 - Architecture and contracts | Approve architecture; decide blockers; define workspace/crate boundaries, PostgreSQL schema/migrations, OpenAPI skeleton, media/worker protocols, capability model, synthetic fixtures, CI and deployment scaffolding | Approved documents; versioned contracts; clean DB migration; test harness; runnable scaffold Compose topology; source dependency license inventory. Seven Rust tests, one SQLx/PostgreSQL 16 migration test, three UI tests/build, OpenAPI validation, schema/constraint checks, and npm audit pass. Platform/image license inventory, worker/telemetry deployment, approved Git remote confirmation, and exit review remain incomplete; Q8 and Q20 are resolved. |
| 1 - Stateful audio slice | PostgreSQL; API/UI; source/destination lifecycle; SRT MPEG-TS, RTP L16/L24, and ALSA 48 kHz stereo L16 inputs; RTP L16/L24 and SRT outputs; one route; local worker registration, scheduler/assignment, health/events and restart recovery | Profile planning and provisional requirement estimates through the API/UI, PostgreSQL-backed source/destination drafts and validation, worker registry, reservation-aware advisory scheduler, atomic reservation and planned assignment workflow, persisted endpoint revision references/configuration snapshots, persistent drain requests, revision-pinned planning API with 24-hour idempotency-key replay, assignment lookup/cancellation APIs with idempotent exact-generation retry, PostgreSQL fence and command ledger, provider-backed bearer verification, local JSON credentials, authenticated WebSocket command dispatch with correlated completion acknowledgements, transport-independent command fencing, worker WebSocket client, generation-gated Start/Stop handlers, typed GStreamer adapter, worker executable with registration/heartbeat/reconnect and post-Welcome assignment reconciliation, PostgreSQL-backed cross-process latest-session fencing, opt-in Compose worker profile, acknowledgement-gated state transitions, assignment-managed source/destination enablement, operator UI controls for a validated single-route assignment, API liveness/readiness endpoints, Compose health checks, and automated API-dispatched RTP MPEG-TS-to-SRT media-plane loopback coverage are implemented and tested. Production secret-store adapters and credential lifecycle, trusted TLS ingress deployment, final GStreamer/plugin license inventory, calibrated resource admission, and multi-route/crosspoint UI remain. |
| 2 - Zephyr Connect | Same-port audio/GPIO demux; parser, bounded ADTS reassembly, AAC decode, baseline/state, API/UI profile, simulator and fixtures | Five-channel synthetic suite/soak; live sender; real output; timing evidence; mock plus native plugin path as available |
| 3 - Routing expansion | Virtualized 64x64 matrix; fan-out; replace; multi-route operations; presets/salvos; take/immediate; locks, concurrency, undo/rollback | UI/API multi-route demos, failure/rollback/concurrency tests, restart recovery |
| 4 - Transport expansion | SRT MPEG-TS and UDP MPEG-TS inputs/outputs; selected HLS/Icecast; ALSA/WASAPI adapters | Capability-aware tests and Linux/Windows deployment documentation |
| 5 - Signals and integrations | Normalized signal engine; SCTE-35/additional approved formats; mappings; GPO; Device Server mock/native plugin; automation command listener | Signal timing, plugin state/reconnect, permissions/audit, device acceptance |
| 6 - Hardening | Backup/restore; security/permission and fault-injection suites; performance/soak; ARM64/Windows validation; operations/install docs | Full definition of done and acceptance report; unresolved gaps explicitly listed |

Each phase maintains runnable integrated software; API, UI, docs, automated
tests, OpenAPI changes and Docker validation are required. The exact Phase 1
demo conflict and Phase 0 blockers are listed in
[open questions](open-questions.md). Do not report a phase complete based only
on its source code or unit tests.

## Git and release workflow

1. The workspace is an existing Git repository on `master`, with no configured
   remote. Confirm the approved ARC remote before any push/release workflow.
   This design task has not created a release branch, phase commit, tag, or push.
2. Work from a protected main branch using short-lived `feature/<scope>`
   branches and reviewed pull requests. Keep generated API/client changes
   traceable to contract changes.
3. At each phase start, create `release/phase-N` from the approved baseline.
   Merge only after required test, docs, OpenAPI, Docker and review gates pass.
4. Update CHANGELOG, version/schema compatibility notes, deployment docs and
   traceability status. Create annotated `phase-N-complete` tag on the
   approved release commit and push branch/tag only after repository
   authorization and CI success.
5. Never claim commits/tags/pushes where the operation did not succeed. Keep
   secrets out of configuration, fixtures, logs and Git history.

## Work breakdown and review

Use small vertical slices. Phase 0 tracks each approved architecture decision
to a contract and test. In each phase, implement domain/application behavior
before adapters, then API/UI, then end-to-end deployment verification. Add
explicit backward-compatibility checks for OpenAPI and database migrations.
Release artifacts include test reports, SBOM/license inventory, deployment
configuration, acceptance evidence and outstanding risks.
