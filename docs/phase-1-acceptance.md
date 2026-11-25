# Phase 1 Acceptance Report

**Status:** Implementation and local acceptance evidence are in progress.

## Implemented and verified

- PostgreSQL migrations, source/destination revisions and assignment persistence.
- Worker registry, heartbeat, resume, reconnect, session leases and generation fences.
- Revision-pinned planning, idempotency, Start/Stop/Cancel lifecycle and resource enablement.
- Authenticated WebSocket dispatch with correlated acknowledgements.
- Typed GStreamer RTP and SRT graphs.
- API-dispatched RTP MPEG-TS-to-SRT live loopback.
- ALSA graph construction test.
- Database-backed readiness and Compose API healthcheck.
- Configured bandwidth guards and OS-derived CPU/memory pressure reporting
  (Linux cgroup v1/v2 or Windows host metrics), with explicit overrides.
- Docker secret-file credential inputs and WSS Nginx ingress POC.
- Optional worker PEM CA bundles, additive to public roots, for private WSS
  ingress trust.
- Clean PostgreSQL migration, API restart, dump and restore script.
- Phase 1 CI workflow and release checklist.
- API, UI and worker Docker images build locally.
- Theme-aware ARC sidebar branding uses optimized 128px PNG derivatives; the
  browser favicon uses the supplied multi-resolution ICO (16/32/48/256px).
  Original artwork is retained under `Reference Docs`.

## CI validation status

The latest GitHub Actions run, [run 45510089696](https://github.com/asalive1/ARC/actions/runs/45510089696)
for commit `4a76825`, passed the UI and Rust unit-test jobs but failed in the API
PostgreSQL/media test and Docker Compose configuration jobs. The API test
expected a conflict for retrying an already-completed cancellation, although
that exact-generation retry is intentionally idempotent; the Compose job also
lacked a non-secret password value for configuration validation. Both issues
are fixed locally. The CI-equivalent Rust unit command now passes with 59 tests,
including Linux live-sample and cgroup v1/v2 telemetry coverage; 14
database-backed tests are ignored in that command. All four ignored API
PostgreSQL/media tests pass serially against an
isolated PostgreSQL 16 service. Local Compose validation and the worker image
build also pass. A successful GitHub Actions run for these fixes remains pending
until a subsequent push or pull request.

The Linux ARM64 worker image also builds with Docker Buildx. Its emulated
runtime reports `aarch64` and contains the worker binary and ALSA, RTP, app,
and SRT GStreamer plugins. This is build/package evidence, not native ARM64
media or device smoke acceptance.

The AMD64 runtime audit confirmed every installed GStreamer plugin reports
LGPL metadata and the required ARC factories load. The emulated ARM64 runtime
also loads the required factories and excludes the known GPL/unknown-plugin
packages. Full OS SBOM/license review and native media smoke tests remain open.

## Acceptance commands

- `cargo test --locked -p arc-domain -p arc-infrastructure -p arc-worker`
- `cargo test --locked -p arc-api`
- UI: `npm test -- --run`, `npx tsc -b`, `npm run build`
- PostgreSQL recovery: `powershell -NoProfile -ExecutionPolicy Bypass -File deploy/validate-postgres-recovery.ps1`
- Docker: `docker compose build api ui worker`

## External acceptance gates

- After Phase 2 implementation, live ALSA hardware produces RTP L16 without
  underrun or device errors, and deployed RTP-to-SRT evidence is reviewed.
- A trusted WSS ingress uses a certificate valid for the worker hostname.
- Production credential provider lifecycle is implemented and tested.
- ARM64/AMD64 Linux and x64 Windows worker targets pass media smoke tests.
- GStreamer/plugin SBOM and package closure exclude GPL/AGPL components and
  resolve all unknown licenses.
- Real CPU, memory, network bandwidth and stream costs are calibrated.
- Multi-route/crosspoint UI is not a Phase 1 implementation requirement.
- Approved remote, commit/release branch, CI, phase tag and push remain external authorization gates.
