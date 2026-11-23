# Component Design

**Status:** Draft for approval

## Component map

| Component | Layer / plane | Main responsibility | Key contracts |
|---|---|---|---|
| Source catalog and revision service | Application / control | Draft, validate, activate, enable, stop, revise, probe and audit sources | `SourceAdapter`, repository |
| Destination catalog and revision service | Application / control | Destination configuration, capability checks, lifecycle and rollback | `DestinationAdapter`, repository |
| Routing service | Domain + application / control | Desired routes, crosspoint edits, take/immediate workflow, replacement, fan-out, locks, presets, batches and rollback | `RouteRepository`, operation service |
| Route planner/admission | Application / control | Validate compatibility, estimate resources and plan before mutation | `WorkerScheduler`, capabilities, `CapacityManager` |
| Operation manager | Application / control | Long-running lifecycle, progress, per-item results, cancellation and idempotency | Operation repository, event publisher |
| Worker registry and lease manager | Application / control | Registration, heartbeat, version/capability/capacity, state, assignment generations and lease expiry | `WorkerRegistry`, `LeaseProvider` |
| Scheduler/capacity manager | Application / control | Compatible, safe-capacity placement; waiting/rejection; local POC adapters | `WorkerScheduler`, `CapacityManager` |
| Reconciler | Application / control | Compare durable desired/assignment state with observed worker/pipeline state; recover after restart/loss | Repository, worker registry, event publisher |
| Network inventory | Infrastructure / worker | Discover stable interface identity, addresses, MTU, link/admin state, multicast and speed | `NetworkInterfaceProvider` |
| Media runtime manager | Infrastructure / media | Create/stop/reconfigure isolated source, route, destination pipelines | `MediaPipeline` |
| Source/destination adapters | Infrastructure / media | Transport-specific capture, decode, encode, output; advertise real capability | `SourceAdapter`, `DestinationAdapter` |
| Signal engine | Domain/application + worker | Parse, normalize, map, deduplicate/debounce, time, dispatch and audit signals | `SignalExtractor`, `SignalGenerator` |
| GPIO integration | Infrastructure / worker | Normalize logical state and execute outputs through selected backend/plugin | Device Server protocol, signal ports |
| API presentation | Presentation / control | REST/OpenAPI, auth middleware, problem details, read-only WSS | Application services |
| Web UI | Presentation / control | Endpoint editors, interface selector, matrix, operations, health, signal views | REST + WSS; TanStack Query |
| Persistence adapter | Infrastructure / control | PostgreSQL access and migrations | `StateRepository`; SQLx |
| Event/telemetry adapter | Infrastructure | Durable/replayable event envelopes as required; logs/metrics/traces | `EventPublisher`, OTel |
| Secret/backup adapters | Infrastructure | Non-revealing secret resolution and backup/restore provider | `SecretProvider`, `BackupProvider` |
| Device Server plugin | Separate deployment | Native plugin ABI and translation to ARC protocol v1 | Versioned authenticated WebSocket JSON |

## Dependency direction

```text
Presentation -> Application -> Domain
Infrastructure -> ports defined at Domain/Application boundary
```

Domain and application compile without Axum, SQLx, GStreamer, platform APIs,
AWS SDKs, or Device Server SDKs. Adapters implement ports at the edges. Shared
DTO/protocol crates contain versioned wire schemas only; they do not grant
callers access to internal domain objects.

## Command and observation flow

1. REST request is authenticated, authorized, bounded, and checked for
   idempotency and expected revision.
2. Application validates domain constraints and returns dry-run warnings or a
   structured rejection before side effects.
3. A short PostgreSQL transaction records operation/desired state and advances
   its revision; media side effects run asynchronously through worker
   assignment and operation lifecycle.
4. Worker verifies assignment generation and capability, applies the command,
   and reports observations/health.
5. Reconciler updates observed state without overwriting desired state, records
   audit/event outcomes, and emits WebSocket updates.

For route replacement, define compensation and partial-failure outcomes;
database transactions cannot make external media pipelines atomic.
