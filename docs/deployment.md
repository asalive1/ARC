# Local Compose Deployment

This Compose stack is a local development deployment of PostgreSQL, the API,
and the operator UI. An opt-in `worker` profile adds the worker service. The
API applies SQLx migrations before opening its HTTP listener. PostgreSQL data
persists in the `postgres_data` named volume.

## Saved Salvo action rollout and recovery

Update the API and UI together for mixed Take/Unroute salvos, then refresh
operator browser tabs before using them. Migration `0014` defaults existing
saved routes to Take and adds persisted `start`/`stop` actions. Red selections
disconnect the output regardless of their saved input anchor.

Before upgrading an existing test stack, back up PostgreSQL and the exact
images, build sources and Compose overlays in a private directory. Wait for
active routing operations to finish. Restart only the API and UI; reload
existing NGINX proxies after replacing the API so their upstream DNS is fresh.
Check readiness, a new worker control session and a fresh heartbeat in a valid
`ready` or `accepting` state. Compare route history, assignment IDs,
generations/states and the command ledger before and after the rollout.

An API rollback binary must embed every applied SQLx migration: the original
13-migration API refuses to start after migration 14 (`VersionMissing(14)`),
even though the column addition is schema-compatible. The test-host backup
includes a separately built and readiness-tested baseline API with migration
14 embedded. Downgrading action-unaware behavior is safe only when no saved
Stop actions or active routing operations remain; otherwise recover forward.
Do not delete migration history or restore an older live database to bypass
these checks.

The mixed-action rollout verified API/UI identities, migration 14, worker
reconnection and unchanged route/assignment/command snapshots. Worker,
database and ingress containers were not recreated, and no media commands
were issued. Audio continuity was not measured during this update; this does
not close external-endpoint or multi-instance/recovery acceptance gates.
Desktop browser checks also verified visible blue/red/clear cycling and that
returning to Salvo clears a mixed draft and restores Select Salvo. No live
salvo Save, Take or Unroute was submitted for these checks.

## Isolated test-server readiness

A Linux Docker/Compose host can run an isolated test stack before all phase
acceptance gates are closed. This is a test deployment, not a production
readiness declaration. Server access, free disk/RAM and a visible ALSA control
device alone do not prove working audio capture/playback.

Before deploying:

1. Review and back up the exact application revision. Use a separate Compose
   project, fresh PostgreSQL volume and unused ports; do not reuse preview
   credentials or an existing database.
2. Provision the database password and administrator bootstrap/recovery
   secret files. The first administrator is created through bootstrap; there
   is no default login. Protect the recovery token offline.
3. Keep the default UI/API loopback bindings for an SSH-tunnel-only preview,
   or configure trusted HTTPS ingress before LAN access. Plain HTTP and
   `ARC_ADMIN_COOKIE_SECURE=false` are for isolated development only.
4. For real routing, enable the worker profile and provision matching worker
   identity/token secrets. Configure a reachable `wss://` endpoint with a
   matching trusted certificate. A private CA can be mounted using
   `ARC_WORKER_CONTROL_CA_FILE`; a public ACME deployment also requires real
   DNS and certificate issuance. Do not treat container `localhost` as the
   host or another service.
5. Plan the configured RTP/SRT UDP ports and firewall/container mappings.
   Explicitly expose audio devices and check capture/playback permissions
   before enabling ALSA. Enumerate actual PCM devices and supported formats;
   `/dev/snd/controlC0` alone is not sufficient.
6. Set conservative worker capacity for the intended test. The default two
   units fit one source plus one destination branch, not a full matrix;
   shared-source fan-out requires one source unit plus one per branch.
   Calibrate limits from measured workloads rather than matrix dimensions.
7. Verify readiness, administrator login, worker registration, one real media
   route, fan-out, replacement, Unroute, salvo execution, restart recovery
   and backup/restore. Record failed or untested gates explicitly.

The UI/API-only preview can precede worker/media setup. Full Phase 2 acceptance
still requires multi-instance/reconnect and retry-exhaustion/manual-recovery
evidence, plus native/deployed media and ALSA validation.

## Operating the SSH-only UI/API preview

The initial preview uses Compose project `arc-ui-preview` in
`~/arc-ui-preview` on the test host. The UI/API-only configuration below
describes its first deployment. **The current test host has since enabled
the [media-worker extension](#linux-dual-network-media-worker); use
`compose-media`, not `compose-preview`, to operate that host.**

- UI: host loopback `127.0.0.1:13000`.
- API: host loopback `127.0.0.1:18080`; the UI also proxies `/api/`.
- PostgreSQL: private Compose network and persistent
  `arc-ui-preview_postgres_data` volume, with no published port.
- Worker: disabled in the initial UI/API-only configuration.

The runtime directory contains the base Compose file, administrator-secret
overlay, image-only preview override and executable `compose-preview` helper.
Images are built on the workstation from an allowlisted working-tree snapshot,
then transferred directly over SSH. `backup/source-manifest.json` records the
baseline commit, individual file hashes and source archive hash; the snapshot
includes uncommitted implementation changes and is not a new Git commit.
`backup/images-manifest.json` records the exact three runtime image identities.
Both archives are retained on the host. No secrets or reference materials are
included in the source/image transfer.

From the workstation, replace `user@test-server` with the supplied SSH target:

```powershell
ssh -N -T -L 127.0.0.1:4180:127.0.0.1:13000 -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 -o ServerAliveCountMax=3 user@test-server
```

Keep that command running and open `http://127.0.0.1:4180`. If a tunnel is
already running, use it rather than starting another on the same local port.
Closing the tunnel does not stop the server. `ARC_ADMIN_COOKIE_SECURE=false`
is limited to this loopback/SSH development setup, not LAN or public HTTP.

On a new deployment, the first visit requires administrator setup. The operator
has completed setup on the current test host; the worker extension preserves
that account and its database. Retrieve a new host's one-time bootstrap
secret in your own terminal, not in chat:

```powershell
ssh user@test-server "cat ~/arc-ui-preview/secrets/admin_bootstrap_token.txt"
```

Enter it in the setup screen and choose your own administrator username and
password. No account or default password is pre-created. Store the separate
`secrets/admin_recovery_token.txt` value securely offline as well. The host
deployment and secrets directories are private (`0700`); the secret files
are mounted read-only and readable by the API's non-root UID `10001`.
Do not delete mounted files or change their readability without updating and
testing the service configuration.

On the test host:

```bash
cd ~/arc-ui-preview
./compose-preview ps
./compose-preview up --no-build -d --wait postgres api ui
# Stop the preview while preserving its database:
./compose-preview stop
```

Use the prebuilt images; do not request `--build` in the runtime directory.
The source archive is retained for rebuilding in a separate clean location.
Do not use `down --volumes` unless permanently destroying this preview database
is intended.

Initial acceptance checks verify 13 successful migrations, an empty catalog,
API/database readiness through the UI proxy, unauthenticated catalog rejection,
the real production administrator-setup screen, and PostgreSQL cluster identity
after a PostgreSQL/API restart. The initial database dump was restored into a
separate temporary database and checked before that temporary database was
removed. `backup/initial-preview.dump` is retained with mode `0600`; it is a
pre-bootstrap snapshot, not an ongoing backup policy. Protect subsequent dumps
like credentials. The existing authenticated operator session survived the
current host's subsequent API/UI update.

## Resource validation and route compatibility

`Active` means the saved configuration is approved for routing; it does not
mean audio is flowing or verify endpoint reachability. New and edited
configurations must be validated before routing. Profile constraints are also
checked when a configuration is saved. Assignment Start/Stop manages resource
enablement, which is not shown as a separate status in the resource list.
The test-server UI was updated independently on 2026-11-27; existing browser
tabs must be refreshed to display the new label. The API, worker, PostgreSQL
and ingress containers were unchanged, and the assignment snapshot remained
unchanged. Audio continuity was not measured.

On 2026-11-27 a further UI-only rollout simplified crosspoint hover text to
input and output names. The API, worker, PostgreSQL and ingress containers and
assignment snapshot were unchanged. Audio continuity was not measured. Refresh
an existing browser tab to load the updated UI. The prior UI image and rollback
materials remain under `backup/ui-hover-20261127-32d118d3`.
The crosspoint direction labels were enlarged and repositioned in a subsequent
UI-only rollout. The worker, API, database, ingress and assignment snapshot
were unchanged; audio continuity was not measured. Its rollback materials are
retained under `backup/ui-matrix-labels-20261127-32d118d3`.
The misleading Shift+F10 hint was removed in another UI-only rollout. The API,
worker, database, ingress and assignment snapshot were unchanged; audio
continuity was not measured. Rollback materials are retained under
`backup/ui-shortcut-20261127-32d118d3`.

An SRT listener binds a local worker address (or an appropriate wildcard),
not the remote sender or receiver address. An SRT caller uses the remote
listener's address. Both peers must use compatible mode and port settings.
After a failed Take, ARC attempts to Stop the failed assignment before
releasing its reservation. An unconfirmed Stop is retried and retains the
reservation; exhausted cleanup requires manual recovery. Existing assignments
from older versions can still occupy an output. If it is safe to disconnect,
Unroute that output and wait for confirmation before trying another Take.
Unroute can interrupt an existing connection. If Stop is not confirmed, do not
manually delete the assignment or release its capacity; investigate worker
connectivity and recovery instead.

Route compatibility is checked separately. Supported paths are ALSA L16 to RTP
L16, RTP L16 to RTP L16, RTP L24 to RTP L24, RTP MP2T to SRT MPEG-TS, and SRT
MPEG-TS to SRT MPEG-TS. There is no automatic L16/L24 conversion or PCM-to-MPEG-TS
encoding. In particular, an ALSA input cannot currently route directly to an
SRT MPEG-TS output. The API rejects unsupported pairs with HTTP 422; the UI
displays the server's explanation alongside that status.

## Linux dual-network media worker

The test host now also runs a non-root media worker and worker-only TLS
ingress. Existing business/audio interfaces were already configured; no host
IP, routing or firewall changes were made. This enables media-route testing;
registration and device access do not themselves prove routed audio.

The Linux-only [host-network overlay](../compose.worker-host.yaml) combines
with the base, administrator-secret and worker-secret overlays. It:

- Gives the worker the host's network namespace, so RTP/SRT can use both
  interfaces without guessing Docker UDP port mappings.
- Publishes PostgreSQL only on host loopback, because the host-network worker
  cannot resolve the private Compose `postgres` hostname.
- Uses loopback WSS with normal certificate/hostname verification and a
  read-only private-CA bundle; TLS verification is not disabled.
- Maps `/dev/snd`, adds the host audio group's numeric GID, and mounts the
  selected ALSA configuration read-only. The worker remains UID/GID
  `10001:10001`, not privileged.

Bind input listeners to the intended local interface address. Use the actual
receiver address for outputs; the host's routes select the outgoing interface.
The local self-address acceptance below does not prove external NIC traversal.

Set these additional variables in the private deployment environment:

```dotenv
ARC_WORKER_DB_PORT=15432
ARC_WORKER_TLS_PORT=18443
ARC_WORKER_ENABLE_ALSA=true
ARC_WORKER_CAPACITY_UNITS=16
ARC_ALSA_GROUP_ID=29
ARC_WORKER_CA_FILE_SOURCE=./deploy/tls/ca.pem
ARC_ALSA_CONFIG_FILE_SOURCE=/home/REPLACE_USER/.asoundrc
```

The example GID must match `getent group audio` on the actual host. Also
provision the matching worker UUID/token files and TLS certificate/key as
described under [Optional worker profile](#optional-worker-profile). Sixteen
logical units are a conservative test budget, not a measured throughput
guarantee; actual CPU/memory telemetry remains enabled.

On the current test host, use its retained image-only overrides and helper:

```bash
cd ~/arc-ui-preview
./compose-media ps
./compose-media up --no-build -d --wait postgres api ui ingress worker
./compose-media logs --tail=50 worker
# Stop services without deleting the database:
./compose-media stop
```

UI/API remain on `127.0.0.1:13000`/`127.0.0.1:18080`. PostgreSQL and worker
TLS bind only `127.0.0.1:15432`/`127.0.0.1:18443`. Keep using the existing
workstation SSH tunnel at `http://127.0.0.1:4180/`; no LAN web listener was
opened. Do not use the older `compose-preview up` helper after enabling media:
it omits the worker credential overlay and can recreate the API without its
authenticated worker dispatcher.

Audio-preview process fencing and reserved-input preview are experimental.
The test host has both enabled and browser preview is available. Earlier
cleanup stalls disconnected worker control; the worker was restarted and
stale reservations were recovered only after the old process exited. The
deployed fix prevents a disconnected SRT destination from blocking shared
sources: media is discarded while disconnected and resumes when the peer
connects. Live ALSA and SRT previews produced audio and cleaned up, and an
idle RTP preview closed cleanly. Browser meters showed nonzero levels for
both active sources. Both RTP outputs carried nonzero audio after rollout.
The SRT destination at `192.168.50.212:9005` returned UDP port-unreachable;
its end-to-end route cannot be verified until the receiver is available.
The API's
`ARC_API_ENABLE_FENCED_PREVIEW=true` selects the experimental fenced relay;
when unset or `false`, the existing active-route-only relay is retained. On
Linux the opt-in worker configuration uses `ARC_WORKER_ENABLE_PREVIEW_FENCING=true`,
`ARC_WORKER_PREVIEW_FENCE_LOCK`, `ARC_WORKER_PREVIEW_HOST_PROC`, and
`ARC_WORKER_PREVIEW_HOST_ID`. All instances of a worker identity must share
one persistent host-local lock inode, a unique uncloned host ID, and a root
host procfs view. Initial enrollment additionally requires
`ARC_WORKER_PREVIEW_FENCE_INITIAL_ENROLLMENT=true` only after confirming every
legacy worker for that identity has stopped; remove that setting immediately
after enrollment. Do not roll back to an unfenced worker without a separately
verified recovery procedure.

Before the transition, `backup/pre-media.dump`, `backup/pre-media.env` and
the original helper were retained privately. The worker/ingress and updated
UI image archives, source snapshots and SHA256 manifests are also retained
under `backup/`. Archives contain no deployment credentials. Treat database
dumps and environment backups as secrets, not public source artifacts.

The test server certificate expires **2026-12-26 16:29:08 UTC**; replace it
before then and restart ingress. The private test CA lasts 90 days. Renew the
CA and worker trust bundle together before its expiry; do not install this
CA globally or bypass a certificate warning.

Verified on the deployed host: API/database readiness, the existing
authenticated UI session, trusted TLS 1.3, a ready authenticated worker with
advancing heartbeats, ALSA device/configuration readability as UID `10001`
with audio GID `29`, both network addresses, and advertised ALSA/RTP/SRT
adapters. The updated UI passes 42 tests and its production typecheck/build.
Idle worker restart/reconnect also passes. External RTP/SRT endpoints,
physical ALSA hardware and live-route restart recovery remain separate checks.

The authenticated production UI created and validated `ARC Test ALSA 440`,
`ARC Test RTP A` and `ARC Test RTP B`. Taking the ALSA input to both local RTP
outputs produced stereo audio on two separately pinned RTP-to-ALSA monitoring
cables. Each output/channel measured RMS `0.56569` and dominant FFT bin
`439.453 Hz` for the expected 440 Hz tone (48 kHz, 32,768-point FFT, tolerance
2 Hz). After Unroute A completed, output B retained those measurements on both
channels. Unroute B then completed; both routes are observed stopped and
worker reservations returned to zero. This proves local deployed fan-out,
not traversal of an external audio-network device.

The three validated example resources remain for operator use. All test-owned
generators and monitoring receivers were stopped; no unrelated process was
terminated. No tone is intentionally left running. The captures and numerical
reports are retained privately under `backup/audio-acceptance-20261126/`.
`backup/media-ready.dump` includes the resulting catalog/history; its archive
readability was checked, not a new full restore drill.

Acceptance exposed an idle-control issue: the database heartbeat did not put
traffic on the WebSocket, so ingress closed idle sessions after 75 seconds.
The worker now sends standard WebSocket Ping frames every 30 seconds; the
existing server Pong handling keeps the connection active. All ten
control-client tests pass, including the idle regression and existing
Start/Stop acknowledgement coverage. The updated deployed worker stayed
connected for 95 idle seconds without a reconnect. The keepalive image/source
manifest and final image-only override are retained in `backup/` alongside
the earlier rollback artifacts.

ALSA is currently an **input** adapter, not a native destination profile.
For loopback acceptance, explicitly pin `CARD,DEVICE,SUBDEVICE`, for example
`plughw:Loopback,1,0` for capture paired with generator playback subdevice
`0`. An external RTP-to-ALSA monitoring bridge must use a different pinned
cable, such as playback/capture subdevice `1`. Otherwise automatic subdevice
selection can silently monitor the original generator or an unused cable
instead of the routed output. Validate both channels' RMS and frequency, not
only packet reception or a meter that suppresses capture errors. The host's
generator currently produces 440/550/660/770 Hz; its separate FFT helper's
400/500/... labels are not acceptance evidence for these tones.

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
