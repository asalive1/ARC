# Delivery Plan and Git Strategy

**Status:** Phase 1 in progress; Phase 0 exit evidence and remaining Phase 1 items are tracked below.

## Phase gates

| Phase | Deliverable scope | Exit evidence |
|---|---|---|
| 0 - Architecture and contracts | Approve architecture; decide blockers; define workspace/crate boundaries, PostgreSQL schema/migrations, OpenAPI skeleton, media/worker protocols, capability model, synthetic fixtures, CI and deployment scaffolding | Approved documents; versioned contracts; clean DB migration; test harness; runnable scaffold Compose topology; source dependency license inventory. Seven Rust tests, one SQLx/PostgreSQL 16 migration test, three UI tests/build, OpenAPI validation, schema/constraint checks, and npm audit pass. Platform/image license inventory, worker/telemetry deployment, approved Git remote confirmation, and exit review remain incomplete; Q8 and Q20 are resolved. |
| 1 - Stateful audio slice | PostgreSQL; API/UI; source/destination lifecycle; SRT MPEG-TS, RTP L16/L24, and ALSA 48 kHz stereo L16 inputs; RTP L16/L24 and SRT outputs; one route; local worker registration, scheduler/assignment, health/events and restart recovery | The API, UI, PostgreSQL, worker, GStreamer, lifecycle, idempotency, reconciliation, session fencing, readiness, secret-file/WSS POC, optional additive worker CA trust, OS-derived CPU/memory telemetry, local Docker stack and CI/release checklist are implemented. The latest GitHub Actions run exposed two failures that have been fixed and locally revalidated; a successful GitHub rerun is still pending. `deploy/validate-postgres-recovery.ps1` provides clean-migration, restart, dump and restore evidence, and `deploy/generate-rust-dependency-inventory.ps1` provides a locked Rust package inventory. Phase 1 exit remains blocked by external acceptance/release gates: live ALSA hardware, production secret-store lifecycle and certificate issuance/rotation/revocation, measured network and workload-cost calibration, Linux/Windows target validation, runtime GStreamer/package license closure and approved Git remote/release workflow. Live ALSA and deployed media acceptance are intentionally sequenced after Phase 2 implementation. Multi-route/crosspoint UI is outside this phase. |
| 2 - Routing expansion | Virtualized 64x64 matrix; fan-out; replace; multi-route operations; presets/salvos; take/immediate; locks, concurrency, undo/rollback | UI/API multi-route demos, failure/rollback/concurrency tests, restart recovery |
| 3 - Zephyr Connect | Same-port audio/GPIO demux; parser, bounded ADTS reassembly, AAC decode, baseline/state, API/UI profile, simulator and fixtures | Five-channel synthetic suite/soak; live sender; real output; timing evidence; mock plus native plugin path as available |
| 4 - Transport expansion | SRT MPEG-TS and UDP MPEG-TS inputs/outputs; selected HLS/Icecast; ALSA/WASAPI adapters | Capability-aware tests and Linux/Windows deployment documentation |
| 5 - Signals and integrations | Normalized signal engine; SCTE-35/additional approved formats; mappings; GPO; Device Server mock/native plugin; automation command listener | Signal timing, plugin state/reconnect, permissions/audit, device acceptance |
| 6 - Hardening | Backup/restore; security/permission and fault-injection suites; performance/soak; ARM64/Windows validation; operations/install docs | Full definition of done and acceptance report; unresolved gaps explicitly listed |

Phase 1 update: Linux AMD64 and ARM64 worker images now build with only the
LGPL SRT plugin copied from a non-shipped extraction stage; the final runtime
does not install the full Debian bad-plugin bundle. The AMD64 plugin audit
reports LGPL metadata for every installed plugin and confirms the required
factories load. The emulated ARM64 runtime also loads the required factories
and excludes the known GPL/unknown-plugin packages. Native media/device smoke
tests and the complete runtime OS SBOM/license review remain open.

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
