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
- Configured bandwidth and CPU/memory placement guards.
- Docker secret-file credential inputs and WSS Nginx ingress POC.
- Clean PostgreSQL migration, API restart, dump and restore script.
- Phase 1 CI workflow and release checklist.
- API, UI and worker Docker images build locally.

## Acceptance commands

- `cargo test --locked -p arc-domain -p arc-infrastructure -p arc-worker`
- `cargo test --locked -p arc-api`
- UI: `npm test -- --run`, `npx tsc -b`, `npm run build`
- PostgreSQL recovery: `powershell -NoProfile -ExecutionPolicy Bypass -File deploy/validate-postgres-recovery.ps1`
- Docker: `docker compose build api ui worker`

## External acceptance gates

- Live ALSA hardware produces RTP L16 without underrun or device errors.
- A trusted WSS ingress uses a certificate valid for the worker hostname.
- Production credential provider lifecycle is implemented and tested.
- ARM64/AMD64 Linux and x64 Windows worker targets pass media smoke tests.
- GStreamer/plugin SBOM and package closure exclude GPL/AGPL components.
- Real CPU, memory, network bandwidth and stream costs are calibrated.
- Multi-route/crosspoint UI is not a Phase 1 implementation requirement.
- Approved remote, commit/release branch, CI, phase tag and push remain external authorization gates.
