# Local Compose Deployment

This Compose stack is a local development deployment of PostgreSQL, the API,
and the operator UI. An opt-in `worker` profile adds the worker service. The
API applies SQLx migrations before opening its HTTP listener. PostgreSQL data
persists in the `postgres_data` named volume.

## Start

Copy `.env.example` to `.env`, replace the local database password with a
URL-safe value, then run:

```powershell
Copy-Item .env.example .env
docker compose up --build --wait
```

Open the UI at `http://localhost:3000`; the API is also available at
`http://localhost:8080/api/v1/health/live`. Both host ports can be changed with
`ARC_UI_PORT` and `ARC_API_PORT` in `.env`. Published ports bind only to
loopback. The database has no published host port.

## Optional worker profile

The worker image and executable are included but are not started by default.
Before enabling the profile, configure a unique `ARC_WORKER_ID`, a matching
64-character hexadecimal `ARC_WORKER_CONTROL_TOKEN` and
`ARC_WORKER_CREDENTIALS_JSON` mapping that ID to the same token, plus a
reachable `ARC_WORKER_CONTROL_ENDPOINT` using `wss://`. The endpoint must be
served by a trusted TLS ingress that forwards the WebSocket request to the API
and overwrites `X-Forwarded-Proto` with `https`. Compose does not create or
trust a development certificate and the worker does not permit plaintext
connections. Keep these values in the ignored `.env`, not in source control.

After configuration, start the worker profile with:

```powershell
docker compose --profile worker up --build --wait
```

The worker connects to PostgreSQL for its registry heartbeat and durable
command-generation fence, and only advertises readiness while its authenticated
control session is connected. `ARC_WORKER_CAPACITY_UNITS` defaults to `2`.
ALSA capability is disabled by default; enabling `ARC_WORKER_ENABLE_ALSA`
requires exposing the host's audio device to the worker container. The GStreamer
adapter advertises only paths whose required plugin factories are installed.

`docker compose down` stops and removes the containers but preserves database
data. `docker compose down --volumes` also permanently removes the local
database volume.

## Scope and limitations

- The default profile runs the API, UI, and PostgreSQL. The opt-in worker
  profile provides a runnable command client and GStreamer adapter. After
  planning an assignment, generation-checked Start and Stop endpoints dispatch
  commands to the worker. The UI can reserve capacity for a validated
  single-route assignment and Start, Stop, or cancel it before it starts. Stop
  retains capacity until worker acknowledgement. Start enables the route's
  source and destination; acknowledged Stop disables each resource only when
  no other active assignment uses it. Multi-route editing and automatic
  recovery reconciliation remain unimplemented.
- The API can create and list disabled source/destination profile drafts and
  validate drafts, and plan compatibility for the supported Phase 1 media
  paths. The UI can configure/validate drafts, check compatibility, and manage
  a single assignment. Resource enabled state follows active assignments;
  operators cannot toggle it independently. Multi-route crosspoints are not
  implemented.
- Compose builds for the host's Docker platform. The Dockerfiles use
  multi-platform base images and avoid platform-specific media packages, but
  Linux AMD64/ARM64 release builds and Windows x86-64 builds have not yet been
  validated.
- This is a local development stack, not a production deployment. API
  authentication, TLS termination, worker networking/device access, backups,
  and production secret management remain out of scope.
- The API enables the worker-control WebSocket route only when
  `ARC_WORKER_CREDENTIALS_JSON` is configured. Compose does not include a TLS
  proxy; the worker profile therefore requires an independently configured
  trusted ingress that terminates WSS, overwrites `X-Forwarded-Proto`, and
  keeps the API reachable only on its trusted network. Production still
  requires a production secret-store adapter.
- The worker image includes GStreamer core, base, good, bad, and ALSA runtime
  packages for the current Phase 1 paths. The selected plugins and complete
  package/license closure require release-time inventory and review.
- Keep `.env` local; it is ignored by Git. The example password is a
  placeholder, not a credential.
