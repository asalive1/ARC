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

The API readiness endpoint is `GET /api/v1/health/ready`; it succeeds only when
PostgreSQL responds. Compose marks the API container healthy after that check.

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

For an ingress using a private CA, set `ARC_WORKER_CONTROL_CA_FILE` to a PEM
bundle path inside the worker container and mount that public CA bundle
read-only. The worker adds its certificates to the bundled public trust roots;
normal certificate-chain and hostname validation remain enabled. The file must
contain at least one valid PEM certificate. The worker reports a configuration
error at startup if it cannot read or parse the bundle. Updating the bundle
requires restarting the worker.

For example, add this to a Compose override after creating the host-side CA
bundle:

```yaml
services:
  worker:
    environment:
      ARC_WORKER_CONTROL_CA_FILE: /etc/arc/trust/ingress-ca.pem
    volumes:
      - ./deploy/trust/ingress-ca.pem:/etc/arc/trust/ingress-ca.pem:ro
```

For the Docker-secrets and TLS ingress POC, generate a certificate/key and
secret files, then start with both Compose files:

```powershell
.\deploy\generate-local-tls.ps1
New-Item -ItemType Directory secrets -Force
@('{"<worker-uuid>":"<64-character-hex-token>"}') | Set-Content secrets\worker_credentials.json
Set-Content secrets\worker_control_token.txt "<64-character-hex-token>"
$env:ARC_WORKER_CREDENTIALS_FILE_SOURCE="./secrets/worker_credentials.json"
$env:ARC_WORKER_CONTROL_TOKEN_FILE_SOURCE="./secrets/worker_control_token.txt"
docker compose -f compose.yaml -f compose.secrets.yaml --profile worker up --build --wait
```

The Nginx ingress forwards WebSocket upgrades to the API and sets
`X-Forwarded-Proto=https`. Use a certificate/trust chain valid for the worker's
configured WSS hostname. For local private-CA testing, provide its CA bundle
with `ARC_WORKER_CONTROL_CA_FILE`; the generated local certificate is not
trusted by default and must not be reused as a production certificate.

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
For Docker, add the host sound device to the worker service override explicitly,
for example:

```yaml
services:
  worker:
    devices:
      - "/dev/snd:/dev/snd"
    group_add:
      - "audio"
```

Worker capacity is reported on each heartbeat. Linux workers use cgroup v1/v2
CPU accounting and memory limits when those controller files are visible;
otherwise they report host-wide CPU and memory utilization. Windows workers
report host-wide utilization. Set the following variables only when a static
operator override is needed; configured CPU and memory ratios take precedence
over measured values:
`ARC_WORKER_INGRESS_BITS_PER_SECOND`,
`ARC_WORKER_EGRESS_BITS_PER_SECOND`,
`ARC_WORKER_CPU_UTILIZATION_RATIO`, and
`ARC_WORKER_MEMORY_UTILIZATION_RATIO`. Bandwidth limits are absolute POC limits
and still require operator calibration. CPU and memory ratios must be from
`0.0` to `1.0`; a ratio at or above `0.90` rejects new placement. If the
operating system cannot provide a metric, that metric remains unknown and its
pressure guard is unbounded; the worker logs a startup warning when no value is
available.

`docker compose down` stops and removes the containers but preserves database
data. `docker compose down --volumes` also permanently removes the local
database volume.

## PostgreSQL clean migration and restore evidence

Run the isolated recovery acceptance script:

```powershell
.\deploy\validate-postgres-recovery.ps1
```

The script creates a fresh PostgreSQL database, starts the API image, waits for
readiness, restarts the API container, dumps the database, restores it into a
second database, and verifies a representative marker. It writes
`artifacts/arc-postgres-recovery.dump` and removes all temporary containers and
network resources afterward.

Generate the locked Rust package inventory for release review:

```powershell
.\deploy\generate-rust-dependency-inventory.ps1
```

This produces JSON and tab-separated package inventories. It does not replace
the required runtime OS/GStreamer license and package closure.

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
- This is a local development stack, not a production deployment. Docker
  secret-file mounting and the WSS ingress override are POC paths; production
  secret-provider lifecycle, certificate automation, worker networking/device
  access, backups, and key rotation remain separate release work.
- The API enables the worker-control WebSocket route only when
  `ARC_WORKER_CREDENTIALS_JSON` is configured. Compose does not include a TLS
  proxy; the worker profile therefore requires an independently configured
  trusted ingress that terminates WSS, overwrites `X-Forwarded-Proto`, and
  keeps the API reachable only on its trusted network. Production still
  requires a production secret-store adapter.
- The worker image includes GStreamer core, base, good, and ALSA runtime
  packages. It copies only the LGPL SRT plugin from the Debian bad-plugin
  package's non-shipped extraction stage and installs the MPL-2.0 SRT runtime
  library. The complete package/license closure still requires release-time
  inventory and review.
- Keep `.env` local; it is ignored by Git. The example password is a
  placeholder, not a credential.
