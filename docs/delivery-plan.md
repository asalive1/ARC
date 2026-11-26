# Delivery Plan and Git Strategy

**Status:** Phase 1 external acceptance remains open. Phase 2 route-state, durable Take/Unroute execution, safe replacement, shared-source fan-out, and saved-salvo CRUD/strict execution are implemented. The prior baseline passed forty-five backend tests and ten ignored PostgreSQL API tests; the worker runtime compiles. GStreamer tests verify two outputs from one source, sibling continuity after stopping a branch, injected bus-error isolation, and real element-generated flow-error isolation without restarting the source. Deployment/native-media validation remains open. Shared-source capacity tests cover concurrent placement, incremental fan-out admission and full-worker rejection. The UI replaces bottom Take controls with crosspoint menus and places Salvo controls above the grid. Forty-two UI tests and the production build pass; fixture-backed browser checks cover catalog CRUD, salvo editing/execution, keyboard access, safe command retry, and a full 64x64 matrix with 99 initially rendered cells and accessible far-corner selections after asynchronous catalog loading. The latest catalog slice adds two passing PostgreSQL deletion API tests and rechecks the existing catalog lifecycle test.

The focused PostgreSQL test
`route_batch_execution_dispatches_and_records_worker_acknowledgement` also
verifies that failed Stop retains the reservation and scheduler capacity until
acknowledged retry, and that concurrent direct Start waits for Unroute before
returning `409 route_removed` without dispatch. OpenAPI passes the official
3.1 schema and local-reference checks. An isolated production UI image builds
and serves its HTML with HTTP 200; those local validation containers/images
were removed. The persistent test-host preview below is intentionally retained.

### SSH-only UI/API test preview

The initial isolated Linux x86_64 preview was deployed as Compose project
`arc-ui-preview`, with a fresh persistent PostgreSQL volume and only
PostgreSQL/API/UI running. Its worker was disabled. Host UI/API ports bind
to loopback only; an SSH tunnel exposes the UI on workstation loopback port
4180. No LAN HTTP port, worker credentials or audio-device mappings were added.

The API and UI production images were rebuilt from an allowlisted 89-file
working-tree snapshot, including the uncommitted Phase 2 code and migration
0013. The baseline commit is `8d5c524`; this deployment does not imply a new
commit or GitHub push. Source/image archives and manifests are preserved in
the private remote deployment backup directory; archive SHA256 values and the
exact imported image identities were checked before startup. Administrator
bootstrap/recovery secrets and the database password were generated only on
the test host, without printing their values.

Acceptance verifies all 13 migrations, an empty catalog, production UI/API
readiness through the SSH tunnel and UI proxy, `401` on an unauthenticated
catalog request, persistent PostgreSQL cluster identity after restart, and
successful dump/restore into a separate temporary database. The temporary
restore database was removed. A real, non-fixture browser check loads the
expected production asset and first-run administrator screen without creating
an account or reading secrets. Initial administrator creation/login remains
for the operator, using their own credentials.

See [preview operation](deployment.md#operating-the-ssh-only-uiapi-preview)
for access, bootstrap-secret retrieval, service controls and backup limitations.
The host has since gained the media-worker extension below. Native/deployed
media, multi-instance/reconnect and exhausted/manual-recovery acceptance
remain open.

### Dual-network media-worker deployment

The existing test deployment now also runs the current production worker and
worker-only NGINX TLS ingress. The Linux host-network overlay preserves the
existing administrator/database and gives the worker both preconfigured
network interfaces. No OS address/routing/firewall changes were necessary.
UI/API, the newly published worker database connection, and TLS ingress remain
loopback-only; operator access continues through the SSH tunnel.

Acceptance verifies trusted TLS 1.3, authenticated worker registration and
advancing heartbeats, ALSA device/configuration access as non-root UID `10001`
with host audio GID `29`, binding to both configured network addresses, and
ALSA/RTP/SRT adapter advertisement. Capacity is 16 logical test units with real
CPU/memory telemetry, not a throughput certification. No actual media-flow
claim is made from these readiness checks.

The updated UI removes its obsolete statement that route activation is not
available. Its production typecheck/build and all 42 UI tests pass. Production
worker/ingress/UI image identities, source snapshots and archive hashes were
verified during direct SSH transfer and retained with a private pre-transition
database/environment backup. This remains an uncommitted working-tree
deployment, not a new release or GitHub push.

Use the retained `compose-media` helper, not the earlier UI-only helper.
See [media-worker operation](deployment.md#linux-dual-network-media-worker)
for ports, certificate expiry, backup details and pinned ALSA subdevices.
Deployed ALSA-loopback-to-RTP fan-out/Unroute measurements now pass through
the authenticated production UI: one 440 Hz ALSA source feeds two stereo RTP
outputs. Each output/channel measures RMS `0.56569` and dominant FFT bin
`439.453 Hz` (48 kHz, 32,768-point FFT, within 2 Hz of expected). After Unroute
A, both channels of B retain those measurements. Final Unroute leaves both
routes observed stopped and worker reservations at zero. The approved example
resources/history remain; only test-owned generators/receivers were stopped.
Captures, reports and a post-media database dump remain in the private backup.

The test also exposed repeated idle WSS disconnects at the 75-second ingress
timeout. Worker Ping frames every 30 seconds fix the root cause without
weakening TLS or increasing the proxy timeout. All ten control-client tests
pass, including idle keepalive and existing Start/Stop handling; the updated
production worker stays connected through a 95-second idle test.

The operator's external RTP/SRT endpoints, physical audio hardware and
live-route recovery remain acceptance work. ALSA output is not a native ARC
destination; the external monitoring bridge is only a test aid. Other Phase 2
gates still include multi-instance execution/reconnect and retry-exhaustion/
manual-recovery evidence.

### Input/output configuration and unused-resource deletion

The resource screens now use a collapsed, full-width creation panel and
full-width expandable rows for inline editing. Enabled resources can be
inspected but cannot be edited or deleted; the visible explanation makes
clear that opening an editor never stops media. Save, Cancel, Validate saved
revision and confirmation-backed Delete are available inside each row.

`DELETE /sources/{sourceId}` and `DELETE /destinations/{destinationId}` require
`expectedRevision`. A transaction locks the resource, checks the revision and
disabled state, and deletes only an unused resource and its configuration
revisions. Existing foreign keys reject route history, assignment, batch,
signal and salvo references; foreign-key conflicts, including commit failures,
are surfaced as `409 resource_in_use`, not silently ignored or cascaded away.
Stale deletion returns `409 stale_resource_revision`; enabled deletion returns
`409 resource_enabled`. No database migration or dependency change is needed.

Two new isolated PostgreSQL API tests cover both resource kinds: stale and
invalid revisions, enabled rejection, successful unused deletion, missing
resources, saved-salvo references, removed-route history and rollback of
configuration-revision cascades after rejected deletion. The existing
PostgreSQL catalog lifecycle test also passes. The UI suite now has 42 passing
tests and its production typecheck/build passes. Fixture-backed desktop browser
checks cover full-width panels, collapsed creation, inline edit/cancel/save,
validation, guarded deletion/confirmation and active-resource protection.

The initial read-only SSH, Docker/Compose and capacity checks were followed by
the isolated UI/API deployment described above. The deployment checklist
distinguishes this preview from worker WSS, media networking and ALSA
acceptance. Full Phase 2 acceptance remains open for multi-instance/reconnect,
exhausted/manual recovery and native/deployed media.

### Replacement cleanup-failure evidence

The PostgreSQL/WebSocket route-execution test now fails cleanup Stop after both
failed Prepare and failed Start. It verifies persisted
`replacement_cleanup_pending`/`replacement_rollback_pending` outcomes, reserved
replacement capacity, scheduler accounting of five units before the old route
is stopped and four afterward, and an unaffected running sibling assignment.
Retried Stop commands keep the same envelope; withholding acknowledgement
prevents restoration and capacity release. Acknowledged cleanup releases the
target reservation, preserves the old route after failed Prepare, and permits
restoring its pinned configuration after failed Start.

This exposed and fixed an admission gap: Unroute rejected pending execution,
but a new Take could supersede a cleanup retry. Desired-route writes now reuse
Unroute's transactional destination guard. New batch Take, preset execution,
direct desired writes and Unroute return `409 destination_busy` during
already-started pending execution; never-started pending requests retain their
existing supersession behavior. Retrying the original accepted batch/key still
returns its original operation. The test exercises all four conflicting write
paths and accepted replay.

The ten ignored API tests pass against a disposable PostgreSQL 16 instance,
alongside 23 related API/infrastructure unit tests, targeted formatting, and
OpenAPI 3.1 schema/163 local-reference checks. This is worker-acknowledgement
failure/retry evidence, not proof of process-loss, retry-exhaustion or native
media teardown behavior. Multi-instance execution/reconnect, exhausted/manual
recovery and deployed/native acceptance remain open.

### Shared-source flow-error evidence

Bounded queues alone do not isolate downstream flow failures: a failed queue
can return its error to the tee and stop the source. Destination ghost sink
pads now contain buffer and buffer-list flow failures and report a
branch-scoped error instead of failing upstream flow. Flushing during branch
teardown likewise does not flush the source. Observation detaches only active
failed branches, preserves the original error, and acknowledges the shared
bin's cached child-error state without cycling its still-playing source.
Unattributed source errors continue to fail all active branches.

The eight GStreamer tests pass in isolated Linux containers with GStreamer
1.22.0 (Debian Bookworm, matching the worker image family) and 1.26.2.
`isolates_an_element_flow_error_before_observation` uses an actual
`identity error-after` failure, verifies ten additional sibling buffers before
controller observation, checks Failed/Running route states and useful failure
details, verifies only one source stream-start event, and cleans up the group.
It also passed ten consecutive runs on 1.26.2. A separate real source-element
failure verifies that both branches fail rather than masking an upstream fault.
The related 45-test Rust suite, ten disposable-PostgreSQL API tests (including
live RTP/SRT dispatch), and worker runtime check pass. Targeted GStreamer
formatting passes; workspace-wide formatting still reports unrelated existing
differences in API startup, worker-domain and worker-authentication files.
These synthetic-media checks do not substitute for native device or deployed
live-media acceptance.

## Phase gates

| Phase | Deliverable scope | Exit evidence |
|---|---|---|
| 0 - Architecture and contracts | Approve architecture; decide blockers; define workspace/crate boundaries, PostgreSQL schema/migrations, OpenAPI skeleton, media/worker protocols, capability model, synthetic fixtures, CI and deployment scaffolding | Approved documents; versioned contracts; clean DB migration; test harness; runnable scaffold Compose topology; source dependency license inventory. Seven Rust tests, one SQLx/PostgreSQL 16 migration test, three UI tests/build, OpenAPI validation, schema/constraint checks, and npm audit pass. Platform/image license inventory, worker/telemetry deployment, approved Git remote confirmation, and exit review remain incomplete; Q8 and Q20 are resolved. |
| 1 - Stateful audio slice | PostgreSQL; API/UI; source/destination lifecycle; SRT MPEG-TS, RTP L16/L24, and ALSA 48 kHz stereo L16 inputs; RTP L16/L24 and SRT outputs; one route; local worker registration, scheduler/assignment, health/events and restart recovery | The API, UI, PostgreSQL, worker, GStreamer, lifecycle, idempotency, reconciliation, session fencing, readiness, administrator bootstrap/recovery, Docker Compose file-backed secrets, worker-token reload/revalidation, optional ACME/WSS ingress, optional additive worker CA trust, OS-derived CPU/memory telemetry, and five-minute strict-FIFO capacity waits are implemented. API, UI and worker images build locally; Rust and UI unit tests pass; administrator bootstrap, CSRF, recovery, login and logout were exercised against an isolated PostgreSQL stack. The PostgreSQL test-fixture failures were fixed, all nine ignored API tests passed locally, and GitHub Actions run 45763988902 passed on commit 8d5c524. `deploy/validate-postgres-recovery.ps1` provides clean-migration, restart, dump and restore evidence, and `deploy/generate-rust-dependency-inventory.ps1` provides a locked Rust package inventory. Phase 1 exit remains blocked by external acceptance/release gates: live ALSA hardware, coordinated worker-secret rotation/revocation deployment testing, real-domain certificate issuance/renewal, the local synthetic resource/network/workload baseline, native Linux and Windows worker validation, GStreamer/package notice and license closure, exact release-image inventory, and approved Git remote/release workflow. Live ALSA and deployed media acceptance are intentionally sequenced after Phase 2 implementation. Multi-route/crosspoint UI is outside this phase. |
| 2 - Routing expansion | Virtualized 64x64 matrix; fan-out; explicit replace; immediate/Take workflows; multi-route operations; route presets/salvos | The matrix separates latest desired and observed route state. Crosspoint Take uses durable `POST /routes/batch` operations; replacements prepare before Stop, perform a hard cut, and attempt restoration of the previous pinned route after failed Start. Each item has leases, stable pipeline/planning IDs, independent outcomes and restart recovery; desired changes are atomic, media switching is not. Shared-source groups have independent bounded destination branches, source-revision affinity, and capacity accounting of one source unit plus one unit per branch. GStreamer tests cover two outputs, stopping one branch without stopping its sibling, injected bus errors and real destination flow errors without source restart; real source errors still fail all branches. Native/deployment evidence remains open. `POST /routes/unroute` removes desired state and durably stops the destination assignment under destination serialization; it retains capacity until acknowledgement and reports `stop_unconfirmed` rather than claiming success when no assignment can confirm Stop. PostgreSQL-backed `/presets` CRUD supports 100 saved salvos of 1-64 distinct assignments with unique 1-100-character names. Strict execution validates the saved preset revision, exact route set and destination revisions captured at selection; accepted execution retries remain idempotent after preset update/deletion. Already-running identical routes are no-ops. The Salvo screen keeps dropdown, Take and Save above its matrix, and left-click staging never switches media. Backend/PostgreSQL and UI suites pass, with fixture-backed browser workflow/full-matrix checks. Saved route removals and Force mode remain unimplemented; Force awaits permission support. Operator locks, real-time multi-user synchronization, granular permissions and user-facing undo/rollback remain deferred. Phase 2 is not declared accepted until its remaining cleanup-failure, multi-instance execution/recovery and native/deployment gates are met |
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
