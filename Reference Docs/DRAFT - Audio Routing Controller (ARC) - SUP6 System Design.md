# ARC Unified Architecture and Stateful Deployment Design

**Document status:** Draft for product, architecture, and engineering review  
**Application:** Audio Routing Controller (ARC)  
**Primary use:** Jira attachment, POC architecture guide, and GitHub Copilot implementation context  
**Applies to:** Windows, Linux, Docker, on-premises, AWS, x86-64, and ARM64 deployments  
**Related specifications:** ARC source handling, destination handling, audio routing and crosspoint control, API and real-time control, and signal/Device Server integration specifications

---

## 1. Purpose

This document defines the unified architecture for ARC across the following concerns:

- Stateful application behavior.
- Portable Windows and Linux deployment.
- Docker deployment.
- x86-64 and ARM64 processor support.
- On-premises deployment.
- AWS production deployment.
- Audio source handling.
- Audio destination handling.
- Audio routing and crosspoint control.
- API and real-time events.
- GPIO, metadata, and signal handling.
- Future elasticity, resilience, upgrades, and worker replacement.

The design must support a POC that can be built and operated without AWS-specific dependencies while establishing interfaces that allow the same ARC domain model to become an elastic, resilient, stateful cloud platform.

---

## 2. Executive Architecture Decision

ARC shall be designed as:

> A stateful platform with durable desired state outside replaceable media-processing runtimes.

ARC shall not be designed as one large server process whose identity, configuration, and active media state exist only on one machine.

The architecture shall separate:

1. **Domain state:** sources, destinations, routes, policies, revisions, mappings, locks, licensing, and audit.
2. **Control plane:** API, validation, scheduling, reconciliation, events, operations, and Device Server coordination.
3. **Media plane:** live input, decoding, normalization, routing, encoding, output, media timing, and cue insertion.
4. **Infrastructure adapters:** persistence, secrets, events, worker management, operating-system integration, and cloud orchestration.

---

## 3. Scope Boundaries

### 3.1 POC scope

The POC shall implement a modular architecture that can operate as one deployable service or a small set of colocated services.

The POC shall not require:

- Amazon ECS.
- AWS managed databases.
- Multi-AZ operation.
- Automatic media-worker scaling.
- Active/standby duplicate media processing.
- Cross-region recovery.
- Kubernetes.
- Cloud-only services.

### 3.2 Production-ready interfaces required in the POC

The POC shall still define and use interfaces for:

- Durable state storage.
- Secret storage.
- Event publication.
- Worker registration.
- Worker capabilities.
- Pipeline assignments.
- Health reporting.
- Backup and restore.
- Media pipeline lifecycle.
- Source and destination adapters.
- Platform audio adapters.

### 3.3 Long-term production scope

The production architecture may separate the modular POC into independently deployable components and add:

- Replicated control-plane services.
- Elastic media workers.
- Worker leases and generations.
- Multi-AZ durable services.
- Application-aware draining.
- Blue/green deployments.
- Warm or active standby media paths.
- Cloud placement and autoscaling.

---

## 4. Architectural Principles

ARC shall follow these principles:

1. Durable desired state is external to media pipelines.
2. Media workers are replaceable.
3. Desired state and observed state are separate.
4. Core business rules do not depend directly on AWS.
5. OS-specific behavior is implemented through adapters.
6. CPU-architecture capabilities are discovered and reported.
7. Source transport is separate from payload/container and codec.
8. Destination transport is separate from payload/container and codec.
9. Routing control is separate from media execution.
10. Signal handling is normalized before mapping or generation.
11. Every mutation is revision-aware, authorized, auditable, and idempotent where retryable.
12. Every long operation is observable and bounded.
13. One failed pipeline must not terminate unrelated pipelines.
14. Slow destinations must not block sources or sibling destinations.
15. Application upgrades must not require in-place mutation of an on-air worker.
16. POC simplicity must not require architectural rework for future cloud deployment.

---

## 5. Logical Architecture

```text
+-------------------------------------------------------------------+
|                         ARC CLIENTS                               |
| Browser UI | Automation API | Device Server | Administrative CLI |
+-------------------------------+-----------------------------------+
                                |
                                v
+-------------------------------------------------------------------+
|                       ARC CONTROL PLANE                           |
| API | Authentication | Validation | Operations | Events | Audit   |
| Route Controller | Reconciler | Capability Catalog | Licensing   |
+-------------------------------+-----------------------------------+
                                |
             +------------------+------------------+
             |                                     |
             v                                     v
+-----------------------------+      +-------------------------------+
| DURABLE ARC STATE           |      | MEDIA WORKER MANAGEMENT       |
| Sources and revisions       |      | Registration and heartbeat    |
| Destinations and revisions  |      | Capabilities and capacity     |
| Desired routes and presets  |      | Assignments and generations   |
| Signal mappings and locks   |      | Drain and replacement state   |
| Operations and audit        |      | Desired/observed reconciliation|
+-----------------------------+      +---------------+---------------+
                                                    |
                                                    v
+-------------------------------------------------------------------+
|                         ARC MEDIA PLANE                           |
| Source Adapters -> Decode/Pass-through -> Route Fan-out           |
|                 -> Process/Delay -> Encode/Mux -> Destinations    |
| Signal Extract -> Normalize -> Map -> Device Server/Output Inject |
+-------------------------------------------------------------------+
```

---

## 6. Portable Domain Model

The following resources shall be deployment-neutral:

- Source.
- Source revision.
- Destination.
- Destination revision.
- Route.
- Route instance.
- Route preset.
- Signal mapping.
- GPIO mapping.
- Secret reference.
- Runtime operation.
- Audit record.
- License entitlement and usage.
- Worker.
- Worker capability.
- Pipeline assignment.
- Assignment lease.
- Health record.
- Backup metadata.

The same API resource model and database migration model shall be used across supported deployments where practical.

---

## 7. State Classification

### 7.1 Durable desired state

The following must survive process, container, host, and application restart:

- Source definitions.
- Destination definitions.
- Draft, validated, and active revisions.
- Desired routes.
- Route presets.
- Locks.
- Signal and metadata mappings.
- GPIO mappings.
- Enabled state.
- Processing policies.
- Delay and cue-offset policies.
- Authentication metadata.
- Secret references.
- Licensing usage.
- Audit records.
- Backup metadata.
- Worker assignment intent when clustered.

### 7.2 Reconstructable observed state

The following may be recalculated or rediscovered:

- Active source sessions.
- Active destination sessions.
- Detected media properties.
- Current health.
- Current worker owner.
- Packet counters.
- Meter values.
- Queue levels.
- Runtime statistics.
- Connected peers.
- Browser monitor sessions.

### 7.3 Ephemeral media state

The following is not treated as durable application state:

- Jitter-buffer contents.
- Partially decoded frames.
- Encoder frames.
- RTP sequence state.
- SRT session internals.
- Delay-buffer audio samples.
- Current socket buffers.
- In-flight cue dispatch.

ARC shall reconstruct pipelines after loss of ephemeral state. Stronger continuity requires warm or active standby pipelines.

---

## 8. Control Plane

The control plane shall provide:

- REST/JSON API.
- Read-only real-time WebSocket events.
- Authentication and authorization.
- Source and destination configuration.
- Route and preset control.
- Signal and GPIO control.
- Validation and dry run.
- Long-running operations.
- Capability discovery.
- Health, statistics, audit, and licensing.
- Backup and restore.
- Worker scheduling and reconciliation in clustered deployments.

Control-plane components shall invoke shared domain services. The browser, public API, and Device Server adapter shall not implement independent business rules.

---

## 9. Media Plane

The media plane shall implement independently restartable pipelines.

```text
Source Receiver
    -> Container/Payload Inspection
        -> Audio Program Selection
            -> Pass-through Eligibility or Decode
                -> Internal 48 kHz float32 Processing Path
                    -> Route Fan-out
                        -> Destination Processing and Delay
                            -> Encode/Packetize/Mux
                                -> Destination Transport or Device
```

Signal path:

```text
Embedded/Sidecar Signal
    -> Extract
        -> Parse and Validate
            -> Normalized ARC Control Event
                -> Route Policy Resolution
                    -> Device Server or Destination Injection
```

Each pipeline shall expose desired state, observed state, health, operation status, and metrics.

---

## 10. Source Architecture

The source model shall separate:

- Identity.
- Lifecycle.
- Network/device connection.
- Transport/session.
- Payload/container.
- Audio program selection.
- Codec and format.
- Media expectations.
- Buffering and latency.
- Processing.
- Signaling.
- Monitoring.
- Recovery.

### 10.1 POC source types

- SRT carrying MPEG-TS.
- UDP carrying MPEG-TS.
- RTP audio.
- HLS.
- Icecast AAC, MP3, and Opus.
- Linux local audio.
- Windows local audio.

Mandatory end-to-end source demonstrations:

- SRT MPEG-TS.
- UDP MPEG-TS.
- RTP.

### 10.2 Internal audio model

- Prefer compatible pass-through.
- Normalize when processing or conversion is required.
- Normalized sample rate: 48 kHz.
- Normalized sample format: 32-bit floating point.
- POC maximum: two channels.
- Default layout: stereo.
- No automatic loudness normalization.
- Gain is explicit.

### 10.3 Source isolation

A source failure or source-local restart shall not restart ARC or unrelated sources and destinations.

---

## 11. Destination Architecture

Each destination shall have its own output contract:

- Codec.
- Sample rate.
- Sample format/bit depth.
- Channel layout.
- Bitrate.
- Packet duration.
- Payload/container.
- Transport or device backend.

### 11.1 POC destination types

- SRT carrying MPEG-TS.
- UDP carrying MPEG-TS.
- RTP audio.
- ALSA output.
- Windows WASAPI shared output.
- Null/test destination.

Additional production design:

- HLS publishing hosted by ARC.
- Icecast publishing to an existing server.
- JACK output where available.

### 11.2 Destination policies

Each destination shall define:

- Prefer pass-through, force encode, or pass-through only.
- No-source behavior.
- Source-failure behavior.
- Startup behavior.
- Output gain and mute.
- Destination audio delay.
- Cue and metadata policy.
- Required downstream confirmation level.
- Retry and recovery policy.

### 11.3 Destination isolation

One slow or failed destination shall not block its source or sibling destinations.

---

## 12. Routing Architecture

Routing shall enforce:

- Zero or one desired source per destination.
- Zero or one active source per destination.
- Unlimited source fan-out within resource limits.
- No mixing in the POC.
- No independent left/right source routing in the POC.

### 12.1 Desired and observed routing

ARC shall persist desired routes and separately report active media flow.

```yaml
route:
  destinationId: destination-id
  desiredSourceId: source-id
  activeSourceId: source-id-or-null
  state: active-or-degraded-or-failed
  revision: 18
```

### 12.2 Route operations

- Create route.
- Explicitly replace route.
- Remove route.
- Fan-out.
- Atomic batch.
- Best-effort batch.
- Preset/salvo execution.
- Undo.
- Rollback.

### 12.3 Route replacement

Replacement shall prepare the new path before disconnecting the old path. If preparation fails, the existing route remains active.

### 12.4 Route persistence

Desired routes shall survive ARC, host, source, and destination restart. Invalid persisted routes remain visible as failed desired routes rather than being silently deleted.

---

## 13. Signal, GPIO, and Metadata Architecture

ARC shall normalize input signals before execution.

Supported policy concepts:

- Pass.
- Validate and pass.
- Regenerate.
- Translate.
- Suppress.
- Consume locally.
- Map to Device Server GPI.
- Generate output signal from Device Server GPO.

Policy precedence:

```text
Route override
    -> destination default
        -> source default
            -> application default
```

Audio delay and cue offset shall remain separate. Signals shall be scheduled against the effective destination media timeline.

---

## 14. API Architecture

The POC shall expose:

- Versioned REST/JSON under `/api/v1`.
- Read-only WebSocket event channel.
- OpenAPI 3.1 contract.
- Interactive documentation.
- TypeScript client SDK.
- C# integration SDK assumption.
- JSON schemas.
- RFC 9457-compatible problem details.

### 14.1 API resource families

- Sources.
- Destinations.
- Routes.
- Route presets.
- Signals and mappings.
- GPIO.
- Operations.
- Health and statistics.
- Capabilities.
- Audit.
- License status.
- Authentication and API keys.
- Secrets.
- System configuration.
- Backup and restore.

### 14.2 Configuration safety

- Disabled draft on creation.
- Explicit validate, apply, enable, and start.
- Immutable resource IDs.
- Friendly names unique by resource type.
- Revisions plus ETag/If-Match.
- JSON Merge Patch and full PUT.
- Unknown write fields rejected.
- Retryable mutations support idempotency keys.

### 14.3 Events

Events shall include a global sequence and event ID. Delivery is at least once within bounded replay history. Clients deduplicate and recover with replay or a fresh snapshot.

---

## 15. Portable Infrastructure Interfaces

The core application shall depend on interfaces rather than cloud SDKs.

```text
StateRepository
RevisionRepository
SecretProvider
EventPublisher
EventReplayStore
ObjectStore
WorkerRegistry
WorkerScheduler
LeaseProvider
OperationQueue
MetricsExporter
PlatformAudioProvider
NetworkInterfaceProvider
BackupProvider
```

Cloud, Docker, Windows, and Linux implementations shall satisfy these contracts.

No core domain service shall contain direct environment branching such as `if deployment == aws`.

---

## 16. Deployment Profiles

### 16.1 Standalone Windows

- Windows service.
- Native WASAPI adapter.
- Network media adapters.
- Local or external database.
- Encrypted local secret provider.
- Self-hosted or external web UI.

### 16.2 Standalone Linux

- Native process or system service.
- ALSA/JACK and optional Linux audio adapters.
- Network media adapters.
- Local or external database.
- Encrypted local secret provider.

### 16.3 Docker on Linux

- ARC container image.
- Persistent database volume or external database.
- Persistent backup/object volume or external store.
- Explicit audio-device mapping when local devices are required.
- Reverse-proxy option.

### 16.4 On-premises clustered

- Multiple control-plane instances.
- Shared durable database.
- Multiple media workers.
- Shared event/coordination service.
- Windows and Linux workers registered by capability.

### 16.5 AWS production

- Replicated control plane.
- Managed durable state.
- Elastic media-worker pools.
- Multi-AZ deployment.
- Blue/green control-plane deployment.
- Application-aware media-worker draining.
- Capacity-aware scheduling.
- Stable protocol-specific media ingress.

### 16.6 Hybrid

- Cloud control plane and network workers.
- On-premises Windows or Linux device workers.
- Secure worker registration and control channel.
- Local hardware access retained on premises.

---

## 17. Operating System and CPU Architecture

### 17.1 Shared builds

Target artifacts:

- Linux x86-64.
- Linux ARM64.
- Windows x86-64.
- Windows ARM64 only when dependency support is validated and required.

### 17.2 Multi-architecture containers

A common image tag may reference Linux AMD64 and ARM64 image variants. Build success does not establish feature parity.

Each build shall publish a capability manifest covering:

- Transports.
- Codecs.
- Encoders.
- Containers/muxers.
- Signal parsers/generators.
- Local audio backends.
- Hardware acceleration where present.

### 17.3 Platform-bound workers

- Linux device worker: ALSA, JACK, PipeWire/PulseAudio as implemented.
- Windows device worker: WASAPI.
- Portable network worker: SRT, UDP, RTP, HLS, Icecast, encode/decode, routing, and signaling.

A Linux container shall not claim WASAPI capability. A Windows worker shall not claim ALSA capability.

---

## 18. POC Runtime Topology

The POC should begin as a modular monolith with separable internal modules.

```text
ARC POC Process/Deployment
  API and WebSocket
  Domain services
  Persistence adapter
  Source manager
  Destination manager
  Routing controller
  Signal/event engine
  Media worker runtime
  Health and telemetry
```

Optional companion services:

- PostgreSQL database.
- Device Server plugin.
- Synthetic media generator/validator.

The POC may run all media pipelines in the same process, but every source and destination pipeline must have an independent lifecycle.

---

## 19. POC Persistence Recommendation

Use a persistence model that can move from standalone to clustered use.

Recommended direction:

- PostgreSQL-compatible durable store for production-capable deployments.
- Optional embedded/local store only for a documented standalone development profile.
- Versioned database migrations.
- Transactional configuration and routing updates.
- Database-backed outbox for reliable event publication.

The database shall store desired state, not high-frequency media samples or meter data.

---

## 20. Worker Resource Model

The POC shall define the worker model even if only one local worker exists.

```yaml
worker:
  id: worker-id
  platform: linux
  architecture: arm64
  deploymentProfile: docker
  version: 0.1.0
  capabilities: {}
  capacity: {}
  state: ready
  drainState: accepting
  lastHeartbeat: 2026-11-08T18:00:00Z
```

Worker states:

- Registering.
- Warming.
- Ready.
- Accepting.
- Draining.
- Drained.
- Degraded.
- Failed.
- Offline.

---

## 21. Assignment and Lease Model

Clustered deployments shall assign pipelines through a generation-controlled lease.

```yaml
assignment:
  id: assignment-id
  resourceType: route-pipeline
  resourceId: route-id
  workerId: worker-id
  generation: 42
  desiredState: running
  observedState: running
  leaseExpiresAt: 2026-11-08T18:01:00Z
```

Rules:

- Only the current generation may operate the assignment.
- Workers renew leases.
- A worker losing its lease must stop stale output.
- Replacement assignment receives a higher generation.
- Desired owner and observed owner are separate.
- Lease loss and split-brain prevention are auditable.

The POC may implement an in-process version of this interface without distributed lease enforcement.

---

## 22. Reconciliation

The reconciler shall compare desired state with observed state.

```text
Desired: Source A enabled and Route A -> Destination B
Observed: No worker reports the active pipeline
Action: Reassign, rebuild, verify, and publish recovery state
```

Reconciliation applies to:

- Enabled sources.
- Enabled destinations.
- Desired routes.
- Worker assignments.
- Presets/operations in progress.
- Device Server connection state.

The POC shall expose reconciliation as a domain concept even if local lifecycle managers perform the work directly.

---

## 23. AWS Production Mapping

Suggested AWS mapping:

- Container orchestration: Amazon ECS.
- Control plane: replicated ECS services, potentially on Fargate.
- Media workers: ECS on EC2 or managed instance capacity for predictable networking and instance selection.
- Durable relational state: managed PostgreSQL-compatible service.
- Secrets: managed secret provider.
- Backups/artifacts: object storage.
- Events/commands: managed queue/event services behind ARC abstractions.
- API/UI ingress: application-aware load balancer.
- Media ingress/egress: network load balancing or stable protocol-specific endpoints.
- Logs/metrics/traces: OpenTelemetry-compatible AWS monitoring stack.

This mapping is a production recommendation, not a POC dependency.

---

## 24. AWS Elastic Capacity

Media capacity shall not scale from CPU alone.

Worker capacity reporting shall include:

- CPU headroom.
- Memory headroom.
- Network headroom.
- Active decoder count.
- Active encoder count.
- Delay-buffer memory.
- Protocol and codec capabilities.
- Current assignment cost.
- Draining state.

Placement flow:

1. Validate resource and route.
2. Estimate media cost.
3. Find a compatible worker with safe headroom.
4. Assign if capacity exists.
5. Request capacity if no placement exists.
6. Wait for a compatible Ready worker.
7. Assign and start the pipeline.
8. Mark active only after observed readiness.

Minimum warm capacity shall be retained for production resilience.

---

## 25. Scale-In and Draining

A live media worker shall not be terminated directly for scale-in.

```text
Accepting
  -> Drain requested
  -> No new assignments
  -> Migrate or stop assignments safely
  -> Verify assignment count is zero
  -> Drained
  -> Safe to terminate
```

Scale-in shall be delayed when a critical pipeline cannot be moved under its resilience policy.

---

## 26. Resilience Levels

ARC shall support a future route/source/destination resilience policy.

### 26.1 Restart recovery

- One active worker.
- Desired state persists.
- Pipeline rebuilt after failure.
- Brief interruption expected.

### 26.2 Warm standby

- One active worker.
- Spare compatible worker capacity reserved.
- Faster rebuild after failure.
- Session interruption may remain.

### 26.3 Active/standby media

- Duplicate critical processing.
- One authoritative output owner.
- Standby validated before takeover.
- Higher compute and network cost.
- Best option for minimal interruption.

The POC implements restart recovery only but shall not block later resilience tiers.

---

## 27. Upgrade and Maintenance Strategy

### 27.1 Control-plane upgrades

Use blue/green or start-before-stop replacement:

1. Start new version.
2. Validate health and API contract.
3. Run synthetic tests.
4. Shift control traffic.
5. Observe.
6. Roll back if required.
7. Stop old version after approval.

### 27.2 Media-worker upgrades

Use application-aware draining:

1. Start new-version workers.
2. Register capabilities.
3. Warm codecs/plugins.
4. Run synthetic media validation.
5. Mark old workers draining.
6. Move eligible pipelines.
7. Verify destination media and cue behavior.
8. Stop old workers only after drained.

### 27.3 OS updates

Use immutable host replacement instead of in-place patching for cloud workers:

1. Build patched image.
2. Start replacement capacity.
3. Validate workers.
4. Drain old hosts.
5. Terminate old hosts.
6. Preserve rollback image for the approved period.

---

## 28. Database Migration Strategy

Use expand, migrate, switch, and contract:

1. Add backward-compatible schema.
2. Deploy code that supports old and new representations.
3. Backfill/migrate data.
4. Switch authoritative behavior.
5. Remove obsolete schema in a later release.

Blue and green application versions may coexist, so database migrations shall not immediately break the previous version.

---

## 29. Stable Media Endpoints

Elastic workers must not require providers to track worker addresses.

```text
External provider
    -> Stable ingress endpoint
        -> Current assignment owner
            -> Media worker
```

Stable ingress may require protocol-specific implementation.

- SRT and TCP may require reconnect during ownership change.
- UDP must preserve packet ordering and sender expectations.
- RTP timestamp/sequence behavior must be tested.
- Fixed provider IP/port requirements must be recorded in source configuration.

The ingress layer shall not become the sole owner of durable ARC state.

---

## 30. Availability-Zone Strategy

Production AWS deployment should use at least two Availability Zones for control-plane and durable services.

Media placement should consider:

- Recovery capacity in another zone.
- Cross-zone transfer cost.
- Source ingress location.
- Destination egress location.
- Latency.
- Resilience policy.

Do not duplicate or transfer high-volume media between zones without an explicit resilience or placement reason.

---

## 31. Security Architecture

- Basic authentication and API keys for the POC.
- OpenID Connect/Keycloak direction for production users.
- OAuth client credentials or scoped API keys for services.
- Secret references instead of returned values.
- TLS through reverse proxy primarily, optional direct TLS.
- Explicit CORS allowlist.
- Trusted proxy addresses.
- Least-privilege scopes.
- Audit for every mutation and override.
- No stack traces or raw secrets in API responses.
- Protected metrics and detailed readiness.

Cloud identity and resource permissions shall be isolated in infrastructure adapters.

---

## 32. Backup and Restore

Backup shall include:

- Configurations and revisions.
- Desired routes.
- Presets and mappings.
- Enabled state.
- Persistent policies.
- Authentication metadata.
- Encrypted secrets only under approved protection.

Backup excludes:

- Runtime audio buffers.
- WebSocket sessions.
- Transient operations.
- In-flight media.
- Unencrypted secrets.

Restore shall validate, dry run, report impact, enter controlled mutation maintenance, apply, and reconcile desired state.

---

## 33. Observability

ARC shall provide:

- Structured logs.
- Metrics.
- Distributed traces.
- Health dimensions.
- Audit history.
- Operation status.
- Global event sequence.
- Worker and assignment health.
- Pipeline resource cost.
- Route switch latency.
- Cue insertion timing.

Every control request should correlate through validation, operation, assignment, media readiness, event publication, and audit.

---

## 34. Failure Isolation

Required isolation boundaries:

- Source pipeline.
- Destination pipeline.
- Browser preview.
- Signal parser/generator.
- Device Server adapter.
- Worker.
- Control-plane process.

A component failure shall be reported and reconciled without terminating unrelated components whenever technically possible.

---

## 35. Repository and Module Structure

Suggested logical repository structure:

```text
/arc
  /domain
  /application
  /api
  /events
  /persistence
  /security
  /routing
  /signals
  /media-core
  /source-adapters
  /destination-adapters
  /platform-linux
  /platform-windows
  /worker
  /deployment-standalone
  /deployment-docker
  /deployment-aws
  /ui
  /device-server-plugin
  /test-harness
  /docs
  /infrastructure
```

The specific programming-language workspace may adjust names while retaining these boundaries.

---

## 36. POC Implementation Constraints

GitHub Copilot shall not implement the full AWS production platform in the POC.

The POC shall:

- Use the portable domain model.
- Use local implementations of infrastructure interfaces.
- Run source/destination pipelines under one local worker runtime.
- Persist desired state.
- Restore desired routes after restart.
- Publish capability data.
- Expose worker and assignment concepts.
- Keep pipeline lifecycle independent.
- Include synthetic generators and validators.
- Include OpenAPI and real-time events.

AWS adapters may be represented as interfaces, configuration schemas, and documented stubs only unless separately authorized.

---

## 37. POC Functional Slice

The initial integrated slice should demonstrate:

1. Create a source as a disabled draft.
2. Validate and apply the source.
3. Create a destination as a disabled draft.
4. Validate and apply the destination.
5. Enable and start both.
6. Create a desired route.
7. Build the active media path.
8. Report desired and observed route state.
9. Pass or normalize audio according to compatibility.
10. Handle a test signal/cue.
11. Persist configuration and desired route.
12. Restart ARC.
13. Recreate pipelines and restore the route.
14. Publish health, operation, and route events.
15. Demonstrate isolated pipeline restart.

---

## 38. Production Evolution Plan

### Phase 1: Portable stateful POC

- Modular monolith.
- Durable desired state.
- Independent pipelines.
- Portable infrastructure interfaces.
- Local worker abstraction.

### Phase 2: Cluster-ready on-premises

- External database.
- Worker registration and heartbeat.
- Distributed assignments.
- Lease/generation enforcement.
- Multiple workers.

### Phase 3: Elastic AWS media plane

- ECS worker manager adapter.
- Capability/capacity placement.
- Scale-out requests.
- Worker draining.
- Minimum warm capacity.
- Stable ingress design.

### Phase 4: Maintenance-aware upgrades

- Blue/green control plane.
- New-version media worker pools.
- Canary migrations.
- Application-aware handoff.
- Automated rollback gates.

### Phase 5: Critical-route resilience

- Warm and active standby policies.
- Cross-AZ recovery.
- Destination takeover.
- Defined recovery objectives.

### Phase 6: Regional recovery

- Cross-region backup/replication.
- Secondary control plane.
- Ingress redirection.
- Regular disaster-recovery exercises.

---

## 39. Architectural Acceptance Criteria

The architecture is accepted when:

- Core domain code has no direct AWS dependency.
- Windows and Linux adapters are separate from domain logic.
- ARM64 and x86-64 capabilities are reported independently.
- Sources, destinations, routes, signals, and API share immutable IDs and revisions.
- Desired state survives process and host restart.
- Media pipelines can be reconstructed from durable state.
- Pipeline restart does not restart ARC.
- One destination failure does not block siblings.
- Worker and assignment interfaces exist in the POC.
- The API exposes desired and observed state.
- Backup/restore reconstructs intended operation without runtime buffers.
- OpenAPI and event contracts are generated and tested.
- POC implementation does not require AWS services.
- AWS production adapters can be added without rewriting business logic.

---

## 40. Required Architectural Test Scenarios

1. Build Linux x86-64 artifact.
2. Build Linux ARM64 artifact.
3. Build Windows x86-64 artifact.
4. Verify capability differences by platform.
5. Persist and restore sources.
6. Persist and restore destinations.
7. Persist and restore desired routes.
8. Rebuild active pipelines after restart.
9. Restart one source pipeline independently.
10. Restart one destination pipeline independently.
11. Fail one destination without blocking siblings.
12. Reject assignment to an incompatible worker.
13. Register a worker and capability manifest.
14. Create a local assignment with generation.
15. Simulate lease loss and prevent stale ownership.
16. Drain a local worker without new assignments.
17. Validate source-to-destination pass-through.
18. Validate automatic conversion.
19. Validate route-specific cue policy.
20. Validate API revisions and idempotency.
21. Replay events after reconnect.
22. Restore from backup without transient state.
23. Run the same domain contract under Windows and Linux adapters.
24. Run synthetic media generator and validator.
25. Verify no core domain package imports AWS-specific SDKs.

---

## 41. Risks and Required Future Decisions

The following require later product or architecture decisions:

- Permitted interruption for ordinary routes.
- Permitted interruption for critical on-air routes.
- Minimum warm capacity.
- Which streams require active/standby duplication.
- Fixed inbound IP/port requirements.
- Required continuity of RTP/SRT/MPEG-TS sessions during migration.
- Cross-AZ and cross-region recovery objectives.
- Database and message-service selection.
- License-provider integration.
- AWS cost constraints.
- Tenant isolation if ARC becomes multitenant.
- Windows cloud-worker need.
- ARM64 codec/plugin parity.

These decisions shall not block the POC unless a selected POC scenario depends on them.

---

## 42. GitHub Copilot Implementation Directives

When this document is supplied to GitHub Copilot in VS Code, the implementation shall:

1. Build the POC, not the full AWS production topology.
2. Preserve the control-plane/media-plane separation in modules.
3. Implement infrastructure-neutral interfaces.
4. Implement local adapters first.
5. Avoid direct AWS SDK calls in domain and application layers.
6. Model worker, capability, assignment, generation, and observed state.
7. Persist source, destination, route, mapping, revision, and operation state.
8. Keep high-frequency media state out of the database.
9. Use independently cancellable and restartable media pipelines.
10. Implement OpenAPI 3.1 and read-only WebSocket events.
11. Use revisions, ETags, If-Match, idempotency, and problem details.
12. Implement source, destination, route, and signal policies from their detailed specifications.
13. Provide Linux and Windows platform adapters.
14. Produce Linux AMD64 and ARM64 container builds where dependencies permit.
15. Report build/runtime capabilities accurately.
16. Include synthetic media and routing test harnesses.
17. Include persistence/restart recovery tests.
18. Include architecture-boundary tests preventing cloud dependencies from leaking into the domain.
19. Mark unimplemented production adapters explicitly.
20. Generate documentation explaining how AWS adapters will attach later.

---

## 43. Definition of Done

This unified architecture document is satisfied for the POC when:

- ARC can be built for the selected Windows and Linux targets.
- Linux container builds are available for selected x86-64 and ARM64 targets.
- The POC operates without AWS dependencies.
- Sources, destinations, routes, API, and signals operate through shared domain services.
- Durable desired state survives restart.
- Active media state is reconstructed from desired state.
- Pipelines are independently managed.
- API, events, backups, capabilities, and health follow the associated specifications.
- Worker and assignment abstractions are present and testable locally.
- Platform-specific audio access is isolated behind adapters.
- The repository and module boundaries support future clustered and AWS deployment.
- Production cloud features are documented without being falsely represented as implemented.
