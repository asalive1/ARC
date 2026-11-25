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
- First-run administrator bootstrap, recovery-token rotation, Argon2 password
  handling, cookie sessions, CSRF enforcement, login/logout and session
  revocation.
- File-backed worker credential reload and active-session revalidation.
- Five-minute strict-FIFO capacity waits with status, cancellation, expiry and
  idempotent retry behavior.
- Optional Certbot ACME issuance/renewal profile and NGINX reload hook.
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
runtime reports `aarch64` and contains the worker binary and required ALSA,
app, audioconvert, coreelements, UDP, RTP, RTP-manager and SRT GStreamer
plugins. This is build/package evidence, not native ARM64 media or device
smoke acceptance.

The AMD64 and emulated ARM64 runtime audits confirmed the required ARC
factories load, all six copied plugin modules are readable to the configured
non-root worker, and the GStreamer registry path is writable. The selected
ALSA, app, audioconvert, coreelements, UDP, RTP, RTP-manager and SRT plugins
report LGPL metadata, and the known unused GPL/unknown-plugin packages are
absent. Fresh worker SBOMs contain 185 package records and 32 `NOASSERTION`
license declarations per architecture. Full OS SBOM/license and notice review
and native media smoke tests remain open.

The local API image SBOM was regenerated (190 packages, 43 `NOASSERTION`
records) and paired with a CycloneDX report of 208 locked API Rust components,
including the administrator-auth dependencies. These are local-build reports,
not release SBOMs. The GStreamer parser header and generated parser outputs are
traced to the shipped core library; the generated outputs contain Bison's
special-exception notice.

The current continuation validation also passed the API, infrastructure and
worker Rust unit suites (18, 5 and 12 tests passed respectively, with 5 API
and 11 infrastructure PostgreSQL-dependent tests ignored in the unit run),
the UI suite (18 tests), and the API/UI/worker image builds. A separate
PostgreSQL-backed administrator flow verified bootstrap, session issuance,
CSRF rejection, recovery-token replacement, prior-session revocation, login
and logout. Capacity PostgreSQL tests and UI integration/build tests passed.
Workspace-wide `cargo fmt --check` is not clean: it reports formatting
differences across existing and modified Rust sources; the new auth and
capacity modules were formatted.

## Acceptance commands

- `cargo test --locked -p arc-domain -p arc-infrastructure -p arc-worker`
- `cargo test --locked -p arc-api`
- UI: `npm test -- --run`, `npx tsc -b`, `npm run build`
- PostgreSQL recovery: `powershell -NoProfile -ExecutionPolicy Bypass -File deploy/validate-postgres-recovery.ps1`
- Docker: `docker compose build api ui worker`

## External acceptance gates

- After Phase 2 implementation, live ALSA hardware produces RTP L16 without
  underrun or device errors, and deployed RTP-to-SRT evidence is reviewed.
- A trusted WSS ingress uses a certificate valid for the worker hostname;
  actual ACME issuance, renewal and deployed reload still require a real
  public hostname and reachable ports.
- Coordinated worker credential rotation/revocation is exercised against a
  deployed Compose stack, including active WebSocket session closure.
- ARM64/AMD64 Linux and x64 Windows worker targets pass media smoke tests.
- Remaining `NOASSERTION` licenses, package notices and base OS obligations
  receive review against exact release artifacts. No published release image
  digests or Windows worker artifacts currently exist; local image IDs are not
  release evidence.
- The reproducible local synthetic CPU, memory, network bandwidth and
  workload-cost baseline is run and documented; hardware/deployed calibration
  remains after Phase 2.
- Multi-route/crosspoint UI is not a Phase 1 implementation requirement.
- Approved remote, commit/release branch, CI, phase tag and push remain external authorization gates.
