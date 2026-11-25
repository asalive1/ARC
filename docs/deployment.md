# Local Compose Deployment

This Compose stack is a local development deployment of PostgreSQL, the API,
and the operator UI. An opt-in `worker` profile adds the worker service. The
API applies SQLx migrations before opening its HTTP listener. PostgreSQL data
persists in the `postgres_data` named volume.

## Start

Copy `.env.example` to `.env`, replace the local database password with a
URL-safe value, and create the first-run administrator secrets:

```powershell
Copy-Item .env.example .env
powershell -NoProfile -ExecutionPolicy Bypass -File .\deploy\new-admin-secrets.ps1
docker compose -f compose.yaml -f compose.admin-secrets.yaml up --build --wait
```

Open the UI at `http://localhost:3000` and enter the one-time bootstrap token
from `secrets\admin_bootstrap_token.txt` to create the administrator account;
there is no default password. Keep the recovery token from
`secrets\admin_recovery_token.txt` offline and protected. After administrator
recovery, replace that file with the newly issued recovery token and recreate
the API service. `ARC_ADMIN_COOKIE_SECURE=false` is only for this local
HTTP-only development setup; use `true` behind production HTTPS.

The API is also available at `http://localhost:8080/api/v1/health/live`. Both
host ports can be changed with `ARC_UI_PORT` and `ARC_API_PORT` in `.env`.
Published ports bind only to loopback. The database has no published host port.

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

For the local Docker-secrets and TLS-ingress POC, generate a certificate/key
and worker credential files, then combine the administrator and worker secret
overlays:

```powershell
.\deploy\generate-local-tls.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\deploy\new-worker-secrets.ps1
docker compose -f compose.yaml -f compose.admin-secrets.yaml -f compose.secrets.yaml --profile worker --profile local-tls up --build --wait
```

Copy the printed worker ID into `.env` as `ARC_WORKER_ID`; set
`ARC_WORKER_CREDENTIALS_FILE_SOURCE=./secrets/worker_credentials.json` and
`ARC_WORKER_CONTROL_TOKEN_FILE_SOURCE=./secrets/worker_control_token.txt`
there as well. Set `ARC_WORKER_NAME` and
`ARC_WORKER_CONTROL_ENDPOINT=wss://localhost:8443/api/v1/workers/control` in
`.env`; the worker UUID must be the same UUID used as the key in
`worker_credentials.json`. The script writes the same randomly generated token
to both files and restricts their Windows ACLs; it never prints the token.

The local Nginx ingress forwards WebSocket upgrades to the API and sets
`X-Forwarded-Proto=https`. Use a certificate/trust chain valid for the worker's
configured WSS hostname. For local private-CA testing, provide its CA bundle
with `ARC_WORKER_CONTROL_CA_FILE`; the generated local certificate is not
trusted by default and must not be reused as a production certificate.

For worker-token rotation, replace both the API credential-map secret and the
worker token secret with the same new 64-character token, then recreate both
services. The API closes an already established session within one second if
the active credential source changes; a recreated worker reloads its token
before reconnecting. There is no overlap period, and the previous token is
rejected. Docker Compose file-backed secrets are snapshots for running
containers, so file changes alone do not update a mounted secret.

After configuration, start the worker profile with:

```powershell
docker compose -f compose.yaml -f compose.admin-secrets.yaml -f compose.secrets.yaml --profile worker --profile local-tls up --build --wait
```

### Public ACME certificates

For production WSS and browser access, configure `ARC_PUBLIC_DOMAIN`,
`ARC_ACME_EMAIL`, `ARC_WORKER_CONTROL_ENDPOINT=wss://<domain>/api/v1/workers/control`,
and `ARC_ADMIN_COOKIE_SECURE=true` in `.env`. Point public DNS at this host and
allow inbound TCP ports 80 and 443. Obtain the initial certificate while port 80
is available:

```powershell
docker compose -f compose.yaml -f compose.acme.yaml --profile acme-bootstrap run --rm certbot-bootstrap
docker compose -f compose.yaml -f compose.admin-secrets.yaml -f compose.secrets.yaml -f compose.acme.yaml --profile worker --profile acme up --build --wait
```

The first command uses Certbot's standalone HTTP-01 challenge and stores the
certificate in the persistent `letsencrypt_data` volume. The second runs the
NGINX HTTPS ingress, serves ACME webroot challenges, proxies the UI/API and
worker WebSocket traffic, and starts a Certbot renewal loop. Successful renewals
signal NGINX to reload its certificate. Check `docker compose logs certbot-renew`
and `docker compose logs ingress-acme` for renewal and reload failures. The
bootstrap command must be repeated for a fresh deployment volume; certificate
issuance and renewal cannot be verified without a real public domain and
reachable ingress.

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
- This is a local development stack, not a production deployment. The API now
  requires first-run administrator bootstrap/recovery secrets and protects
  operator routes with CSRF-checked sessions. Worker credentials can be read
  from Docker Compose secret files and are revalidated on active sessions.
  Compose file-backed secrets are not encrypted at rest; production still
  requires host access controls, protected backups, a managed secret store, and
  controlled rotation.
- The worker profile uses the local self-signed TLS ingress only for testing.
  Production requires a public domain, ACME certificate issuance/renewal,
  trusted ingress configuration, worker networking/device access, and proof
  that API access is restricted to that ingress.
- The worker runtime installs GStreamer core, the Base support library and
  ALSA plugin, plus the MPL-2.0 SRT runtime library. It copies only the
  required app, audioconvert, UDP, RTP, RTP-manager and SRT plugin modules from
  non-shipped Debian extraction stages; none of the full Base/Good/Bad plugin
  bundles are installed in the runtime image. AMD64 and emulated ARM64 checks
  confirm required factories load and unused DV/CACA packages are absent.
  Full package/license and notice review remains a release gate.
- Keep `.env` local; it is ignored by Git. The example password is a
  placeholder, not a credential.
