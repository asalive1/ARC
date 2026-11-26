# Delivery Plan and Git Strategy

**Status:** Phase 1 external acceptance remains open. Phase 2 route-state, durable Take execution, safe replacement, and shared-source fan-out are implemented. GStreamer integration tests verify one RTP MPEG-TS source feeding two SRT destinations, sibling continuity after stopping a branch, and isolation of a deterministically injected branch-attributed bus error while its sibling continues receiving media. Element-generated flow-error behavior and deployment/native-media validation remain unverified. The UI shows desired-output counts per input with independent destination status. Shared-source capacity accounting is implemented and its isolated PostgreSQL test covers concurrent placement, incremental fan-out capacity, and full-worker rejection despite another worker having spare capacity. Route presets/salvos remain.

## Phase gates

| Phase | Deliverable scope | Exit evidence |
|---|---|---|
| 0 - Architecture and contracts | Approve architecture; decide blockers; define workspace/crate boundaries, PostgreSQL schema/migrations, OpenAPI skeleton, media/worker protocols, capability model, synthetic fixtures, CI and deployment scaffolding | Approved documents; versioned contracts; clean DB migration; test harness; runnable scaffold Compose topology; source dependency license inventory. Seven Rust tests, one SQLx/PostgreSQL 16 migration test, three UI tests/build, OpenAPI validation, schema/constraint checks, and npm audit pass. Platform/image license inventory, worker/telemetry deployment, approved Git remote confirmation, and exit review remain incomplete; Q8 and Q20 are resolved. |
| 1 - Stateful audio slice | PostgreSQL; API/UI; source/destination lifecycle; SRT MPEG-TS, RTP L16/L24, and ALSA 48 kHz stereo L16 inputs; RTP L16/L24 and SRT outputs; one route; local worker registration, scheduler/assignment, health/events and restart recovery | The API, UI, PostgreSQL, worker, GStreamer, lifecycle, idempotency, reconciliation, session fencing, readiness, administrator bootstrap/recovery, Docker Compose file-backed secrets, worker-token reload/revalidation, optional ACME/WSS ingress, optional additive worker CA trust, OS-derived CPU/memory telemetry, and five-minute strict-FIFO capacity waits are implemented. API, UI and worker images build locally; Rust and UI unit tests pass; administrator bootstrap, CSRF, recovery, login and logout were exercised against an isolated PostgreSQL stack. The latest GitHub Actions run exposed two failures that have been fixed and locally revalidated; a successful GitHub rerun is still pending. `deploy/validate-postgres-recovery.ps1` provides clean-migration, restart, dump and restore evidence, and `deploy/generate-rust-dependency-inventory.ps1` provides a locked Rust package inventory. Phase 1 exit remains blocked by external acceptance/release gates: live ALSA hardware, coordinated worker-secret rotation/revocation deployment testing, real-domain certificate issuance/renewal, the local synthetic resource/network/workload baseline, native Linux and Windows worker validation, GStreamer/package notice and license closure, exact release-image inventory, and approved Git remote/release workflow. Live ALSA and deployed media acceptance are intentionally sequenced after Phase 2 implementation. Multi-route/crosspoint UI is outside this phase. |
| 2 - Routing expansion | Virtualized 64x64 matrix; fan-out; explicit replace; immediate/Take workflows; multi-route operations; route presets/salvos | The matrix reads the latest route state per destination. `POST /routes` validates endpoint profiles and atomically records desired state using expected revisions; the Immediate/Take execution path uses `POST /routes/batch` for durable worker operations. Batch routes pin endpoint revisions and stable pipeline IDs, execute under leases, persist independent outcomes, retry transient failures, and recover after API restart. A confirmed replacement captures the active assignment, sends a worker Prepare while the prior route remains active, stops the prior assignment only after preparation succeeds, then starts the replacement with hard-cut semantics. A failed preparation keeps the prior route active; a failed replacement Start triggers a durable compensation attempt using the previous pinned route. Recovery progress and outcomes are visible per destination; the operation reports when automatic restoration exhausts its retry budget. Best-effort sibling routes remain independent, and older desired-only batch records are not auto-executed. Acknowledged Start/Stop transitions write route observations transactionally. Shared-source execution groups routes by source revision on one worker and gives each destination an independent bounded branch; integration tests verify two outputs, sibling continuity after stopping one branch, and isolation of a deterministically injected branch-attributed bus error while the sibling continues receiving. Capacity admission counts one source unit per shared source revision plus one unit per destination branch; an isolated PostgreSQL test verifies concurrent affinity, incremental capacity, and rejection when the pinned worker is full. Element-generated flow-error behavior and deployment/native-media validation remain unverified. Preset CRUD and strict execution remain; Force preset mode awaits permission support. Locks, multi-user concurrency, granular route permissions, and user-facing undo/rollback are deferred by the user's scope selection |
| 3 - Zephyr Connect | Same-port audio/GPIO demux; parser, bounded ADTS reassembly, AAC decode, baseline/state, API/UI profile, simulator and fixtures | Five-channel synthetic suite/soak; live sender; real output; timing evidence; mock plus native plugin path as available |
| 4 - Transport expansion | SRT MPEG-TS and UDP MPEG-TS inputs/outputs; selected HLS/Icecast; ALSA/WASAPI adapters | Capability-aware tests and Linux/Windows deployment documentation |
| 5 - Signals and integrations | Normalized signal engine; SCTE-35/additional approved formats; mappings; GPO; Device Server mock/native plugin; automation command listener | Signal timing, plugin state/reconnect, permissions/audit, device acceptance |
| 6 - Hardening | Backup/restore; security/permission and fault-injection suites; performance/soak; ARM64/Windows validation; operations/install docs | Full definition of done and acceptance report; unresolved gaps explicitly listed |

Phase 1 update: Linux AMD64 and ARM64 worker images now copy only the required
app, audioconvert, UDP/RTP, RTP-manager and SRT plugin modules from non-shipped
Debian extraction stages; the final runtime installs none of the full
Base/Good/Bad plugin bundles. Required factories load on both architectures,
and the selected ALSA, app, audioconvert, coreelements, UDP, RTP,
RTP-manager and SRT plugins report LGPL metadata. Unused DV/CACA dependencies
are absent. Docker Scout SPDX inventories cover API, UI, PostgreSQL and worker
AMD64 images, plus the ARM64 worker. Each worker SBOM has 185 package records
and 32 `NOASSERTION` license declarations; package-license and notice review
against exact release artifacts remains open. The API local-image SBOM has
been regenerated and paired with a locked Rust dependency CycloneDX report.
The GStreamer parser-to-core-library mapping and Bison exception are verified.
No release images or Windows worker artifacts currently exist to inventory.
Native media/device smoke tests also remain open.

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
