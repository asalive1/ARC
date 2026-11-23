# ARC API and Real-Time Control Interface Specification

**Document status:** Draft for product and technical review  
**Application:** Audio Routing Controller (ARC)  
**Document role:** Jira attachment, engineering requirements, POC scope, integration contract, and AI-assisted implementation specification  
**Related documents:** ARC Epic, ARC Audio Input Source Handling Specification, ARC Audio Output Destination Handling Specification, ARC Audio Routing and Crosspoint Control Specification, and ARC Signal Handling and WideOrbit Device Server Integration Specification

---

## 1. Purpose

This document defines the external and internal control interfaces for ARC. The API is the common control plane for:

- Source configuration and runtime operations.
- Destination configuration and runtime operations.
- Audio routes and crosspoints.
- Route presets.
- Signal mappings and signal generation.
- GPIO state and commands.
- Health, statistics, capabilities, and licensing.
- Audit history.
- Authentication administration.
- System configuration.
- Configuration and persistent-state backup and restore.
- Real-time event subscriptions.

The browser UI, external integrations, automation systems, and the Device Server adapter shall use the same underlying domain rules. No interface may bypass validation, permissions, locks, revision control, licensing, or resource admission.

---

## 2. POC Scope

The POC API shall include:

- Sources.
- Destinations.
- Routes and crosspoints.
- Route presets.
- Signal mappings.
- GPIO state and commands.
- Health and statistics.
- Runtime operations.
- Event subscriptions.
- Licensing status.
- Audit/history.
- Authentication administration.
- System configuration.
- Plugin and capability discovery.
- Backup and restore.

### 2.1 Backup interpretation

Complete backup/restore is required for the POC. For this specification, it means:

- Configuration.
- Draft and active revisions.
- Desired routes.
- Presets and mappings.
- Enabled states.
- Persistent policies.
- Audit-policy configuration.
- Encrypted secret store or securely transferable secret references when supported.
- Other persistent operational state needed to reconstruct intended behavior.

It shall not include:

- Runtime media buffers.
- Current WebSocket sessions.
- Transient operations.
- In-flight media.
- Unencrypted secret values.
- Runtime queue contents.

---

## 3. API Architecture

ARC shall expose:

1. A versioned REST/JSON API for queries and mutations.
2. A read-only WebSocket event channel for real-time updates.
3. OpenTelemetry/Prometheus-compatible metrics.
4. Specialized Device Server integration invoking the same domain services.

```text
Browser UI ------- REST + WebSocket --------+
External clients - REST + WebSocket --------+--> API layer
Automation ------- REST --------------------+       |
Device Server ---- specialized adapter ------+       v
                                              Domain services
                                                   |
                       source | destination | routing | signal | GPIO
                                                   |
                                              persistent stores
```

Mutations shall remain in REST. The WebSocket channel is read-only.

---

## 4. API Contract and Documentation

OpenAPI 3.1 shall be the authoritative HTTP API contract.

ARC shall produce:

- OpenAPI 3.1 document.
- Interactive API documentation.
- Downloadable JSON schemas.
- Generated client SDKs.
- Example request collection.
- Valid and error examples.
- Asynchronous-operation examples.
- Event-envelope examples.

Generated documentation shall be part of the build and validated in automated tests.

---

## 5. Base URL and Versioning

Base URL pattern:

```text
/api/v1/{resource}
```

Examples:

```text
/api/v1/sources
/api/v1/destinations
/api/v1/routes
/api/v1/operations
```

The POC supports `v1` only.

### 5.1 Version information

API and application build versions shall be available through:

- Response headers.
- System information endpoint.
- OpenAPI document metadata.

They shall not be duplicated in every response body.

### 5.2 Breaking changes

Breaking changes require a new major URL version. ARC shall publish a compatibility and deprecation policy and maintain the previous major version during a documented transition period.

---

## 6. Resource Naming and Identifiers

Resources shall use plural nouns. Lifecycle or command behavior shall use operation subresources.

```text
POST /api/v1/sources
POST /api/v1/sources/{sourceId}/operations/probe
```

Resource identifiers shall be server-generated opaque UUIDs or ULIDs.

Friendly names shall be unique within each resource type. Relationships shall always use immutable IDs.

Enum values shall use lowercase kebab-case.

---

## 7. Common Resource Metadata

Persisted resources should expose applicable common fields:

```json
{
  "id": "opaque-id",
  "friendlyName": "Studio Feed",
  "revision": 7,
  "createdAt": "2026-11-08T18:00:00Z",
  "modifiedAt": "2026-11-08T18:05:00Z",
  "createdBy": "principal-id",
  "modifiedBy": "principal-id",
  "deleted": false
}
```

All timestamps shall be RFC 3339 UTC strings.

---

## 8. Create and Activation Behavior

Source and destination creation shall produce a disabled draft by default.

- `201 Created` for synchronous resource creation.
- `Location` header points to the new resource.
- Activation and enablement are explicit operations.
- Creating a resource shall not implicitly start media.

Create mutations shall support idempotency keys.

---

## 9. Configuration Revision Model

Sources, destinations, signal mappings, and route presets shall support:

- Draft revision.
- Validated revision.
- Active revision.
- Revision history.
- Rollback.

Every update requires an expected revision.

The API shall expose draft, active, and effective runtime representations separately.

---

## 10. HTTP Concurrency Control

ARC shall use:

- `ETag` on revisioned resources.
- `If-Match` for updates and destructive operations.
- Explicit revision fields in request and response models.

Status behavior:

- `412 Precondition Failed` when `If-Match` fails.
- `409 Conflict` for valid HTTP preconditions but conflicting domain state.
- Last-write-wins is prohibited.

Conflict responses shall include the current revision and safe current-state details.

---

## 11. Update and Patch Semantics

ARC shall support:

- Full `PUT` replacement.
- `PATCH` with JSON Merge Patch.

Rules:

- Unknown fields on writes are rejected.
- Omitted fields in Merge Patch are unchanged.
- `null` clears only a nullable field.
- Empty string is not equivalent to null.
- Read-only fields supplied on write are rejected.
- Unsupported enum values are rejected on writes.
- Clients should tolerate unknown enum values on reads.

---

## 12. Delete Semantics

Default deletion is soft delete followed by configurable retention cleanup.

For sources or destinations with routes, deletion requires:

- `If-Match`.
- Expected revision.
- Explicit cascade flag.
- Explicit confirmation.
- Impact preview.

Synchronous completed delete returns `204 No Content`. Asynchronous delete returns `202 Accepted` with an operation resource.

---

## 13. Authentication Architecture

### 13.1 POC

- Basic authentication for browser administration.
- API keys for service integration.
- Interfaces prepared for OpenID Connect.

### 13.2 Production direction

- OpenID Connect through Keycloak for users.
- OAuth client credentials or scoped API keys for services.
- Local emergency administrator mechanism.
- Optional mutual TLS where required.

Authentication mechanisms shall be configurable without changing domain services.

---

## 14. API Key Management

API keys shall include:

- ID.
- Name.
- Owner.
- Scopes.
- Description.
- Creation time.
- Expiration.
- Last-used time.
- Allowed IP ranges.
- Revocation state.
- Rotation metadata.

The key value shall be displayed only once at creation or rotation. ARC shall store a secure verification representation rather than a retrievable plaintext value.

---

## 15. Permissions and Scopes

Scopes shall include:

- `sources.read`
- `sources.write`
- `sources.operate`
- `destinations.read`
- `destinations.write`
- `destinations.operate`
- `routes.read`
- `routes.write`
- `routes.override-lock`
- `presets.read`
- `presets.write`
- `presets.execute`
- `signals.read`
- `signals.write`
- `signals.execute`
- `health.read`
- `audit.read`
- `system.read`
- `system.write`
- `license.read`
- `secrets.write`
- `admin`

The POC administrator role may hold every scope, but enforcement shall be implemented internally.

---

## 16. Secret API

Secret endpoints shall support:

- Create.
- Rotate/replace.
- Delete.
- Read metadata.

They shall never support revealing the stored value.

Configuration references shall use:

```json
{
  "secretRef": "secret-id",
  "configured": true
}
```

Secrets shall be excluded or securely transformed in export and backup processes.

---

## 17. Correlation and Tracing

ARC shall accept:

- `X-Correlation-ID`.
- Standard trace context.

If no correlation ID is provided, ARC shall generate one and return it.

Correlation ID shall appear in:

- Responses.
- Problem details.
- Operations.
- Events.
- Logs.
- Audit records.
- Traces.

---

## 18. Idempotency

All safely retryable mutation operations shall support idempotency keys, including:

- Resource creation.
- Route create/replace/remove.
- Batch routing.
- Preset execution.
- Signal injection.
- Start/stop/restart.
- Probe and tests.
- Test tone.
- Backup/restore initiation.

Retention shall be configurable by operation class.

An identical request retried with the same key returns the original result. Reusing a key with a different request payload is rejected.

---

## 19. Asynchronous Operations

Operations that may exceed a short HTTP deadline shall return `202 Accepted` and an operation resource.

Examples:

- Source probe.
- Source start/stop/restart.
- Destination tests.
- Destination start/stop/restart.
- Route batches.
- Preset execution.
- Signal generation.
- Backup/restore.
- Asynchronous delete.

The response shall include a `Location` header for the operation.

---

## 20. Operation Resource

An operation shall expose:

- Operation ID.
- Type.
- Status.
- Progress.
- Requested by.
- Request time.
- Start and end time.
- Target resources.
- Per-item results.
- Warnings.
- Errors.
- Correlation ID.
- Cancellable flag.
- Expiration time.
- Commit-point status where appropriate.

Common states:

- `queued`
- `validating`
- `running`
- `partially-completed`
- `completed`
- `failed`
- `cancel-requested`
- `canceled`
- `timed-out`
- `rolling-back`
- `rolled-back`

---

## 21. Operation Cancellation

Every operation shall report whether it is cancellable.

Applicable operations include:

- Source probe.
- Destination test.
- Pending batch.
- Pending preset.
- Pending signal.
- Restart before irreversible commit.
- Backup/restore where safe.

Cancellation shall not be offered after an irreversible commit point. Cancel requests may themselves complete asynchronously.

---

## 22. HTTP Status Semantics

Examples:

- `200 OK`: successful read or mutation returning content.
- `201 Created`: resource created.
- `202 Accepted`: asynchronous operation started.
- `204 No Content`: completed operation with no response body.
- `400 Bad Request`: malformed syntax or invalid basic request.
- `401 Unauthorized`: missing or invalid authentication.
- `403 Forbidden`: authenticated but not permitted.
- `404 Not Found`: resource not found or hidden by policy.
- `409 Conflict`: conflicting domain state.
- `412 Precondition Failed`: failed ETag/precondition.
- `413 Content Too Large`: request exceeds size limit.
- `422 Unprocessable Content`: structurally valid but semantically invalid request.
- `429 Too Many Requests`: rate limit exceeded.
- `503 Service Unavailable`: transient maintenance or unavailable dependency preventing the operation.

---

## 23. Problem Details

ARC shall use RFC 9457-compatible problem details with content type `application/problem+json`.

Extension fields shall include when applicable:

- Stable ARC error code.
- Correlation ID.
- Diagnostic reference.
- Field errors.
- Retryable flag.
- Current resource revision.
- Current route state.
- Corrective actions.
- Underlying component identifier.
- Per-item batch errors.

```json
{
  "type": "https://errors.example.invalid/arc/route-conflict",
  "title": "Route state conflict",
  "status": 409,
  "detail": "The destination route changed after it was read.",
  "instance": "/api/v1/routes/destination-1",
  "arcCode": "route-revision-conflict",
  "correlationId": "corr-123",
  "retryable": false,
  "currentRevision": 18
}
```

Ordinary API responses shall never include stack traces or raw library errors. They shall provide a diagnostic reference for authorized log correlation.

---

## 24. Validation and Dry Run APIs

Resources shall support:

- Validate a stored draft.
- Validate a supplied object without saving.
- Dry-run activation.
- Compatibility check against another resource.

Examples:

```text
POST /api/v1/sources/validate
POST /api/v1/sources/{sourceId}/operations/validate
POST /api/v1/routes/dry-run
```

Validation shall return errors, warnings, capability requirements, resource estimates, and corrective actions.

---

## 25. Pagination

Collection endpoints shall use cursor pagination.

Small bounded collections may allow explicitly documented unpaged responses. Audit, operations, events, and history shall always be pageable.

Responses shall include:

- Items.
- Next cursor when present.
- Optional previous cursor where supported.
- Page size.
- Applied sort.
- Applied filters.

No API shall require clients to infer cursor structure.

---

## 26. Filtering and Sorting

Applicable collection endpoints shall support documented filters for:

- Name.
- Enabled state.
- Runtime state.
- Health.
- Protocol/type.
- Environment.
- Role.
- Tags.
- Modified time.
- Routed/unrouted.
- Deleted state.

Sorting shall support documented fields and ascending/descending direction. Unsupported filters or sort fields shall be rejected.

---

## 27. Expansion

ARC shall support a documented `include` or `expand` mechanism for applicable resources:

- Health.
- Active revision.
- Route information.
- Related resources.

Sparse fieldsets are deferred unless payload size becomes a demonstrated concern.

Expansion depth and response size shall be bounded.

---

## 28. Snapshot and Bootstrap APIs

ARC shall provide:

- Full source snapshot.
- Full destination snapshot.
- Full route snapshot.
- Combined operational snapshot.
- Capability snapshot.
- Browser bootstrap endpoint.

The bootstrap response shall include revision markers and event sequence information so the UI can transition safely to incremental WebSocket updates.

---

## 29. Real-Time WebSocket Interface

The POC shall provide a read-only WebSocket channel.

Mutations remain in REST.

Authentication and scope enforcement shall apply during connection and subscription.

Required topics:

- Sources.
- Destinations.
- Routes.
- Operations.
- GPIO.
- Signals.
- Health.
- Statistics.
- Audit.
- Licenses.
- System.

---

## 30. Event Envelope

Each event shall contain:

```json
{
  "eventId": "event-id",
  "topic": "routes",
  "eventType": "route-active",
  "schemaVersion": "1.0",
  "globalSequence": 10452,
  "resourceId": "route-id",
  "resourceRevision": 18,
  "timestamp": "2026-11-08T18:10:00Z",
  "correlationId": "corr-123",
  "origin": "api",
  "traceContext": {},
  "payload": {}
}
```

Events shall be permission-filtered.

---

## 31. Event Delivery and Replay

POC guarantees:

- At-least-once delivery while connected or during replay.
- Global sequence.
- Event IDs for deduplication.
- Replay within a bounded retention window.
- Globally ordered publication by sequence.

Clients shall deduplicate by event ID and process sequence gaps explicitly.

### 31.1 Reconnect

A reconnecting client may provide its last global sequence.

- Replay events if retained.
- Require a fresh snapshot if replay history is unavailable.
- Return the new sequence boundary after snapshot.

---

## 32. Statistics Subscriptions

Fast-changing statistics shall use per-resource or topic subscriptions with configurable intervals and server-enforced minimums.

Defaults:

- One second for meters and operational values.
- Slower intervals for administrative metrics.

The API shall prevent abusive subscription cardinality and update frequency.

---

## 33. Health APIs

ARC shall expose:

- Process liveness.
- Application readiness.
- Dependency health.
- Detailed component health.
- Source health.
- Destination health.
- Version/build information.

Minimal liveness may be unauthenticated. Detailed readiness and dependency information shall require authorization.

Media-pipeline failures shall not make the control API unavailable unless overall application health is compromised.

---

## 34. Metrics Endpoint

ARC shall expose OpenTelemetry/Prometheus-compatible metrics.

Access shall require authentication or network allowlisting except in explicitly isolated deployments.

Metrics shall not expose secrets, credentials, raw sensitive payloads, or unbounded identifying labels.

---

## 35. Audit API

Audit records shall be:

- Read-only through ordinary APIs.
- Filterable.
- Cursor paginated.
- Immutable.
- Retained by time and storage limits.
- Exportable in the production design.

POC requirement: read and filter. Audit export is post-POC.

---

## 36. Licensing API

The POC shall expose non-sensitive:

- License status.
- Effective entitlements.
- Current usage.
- Remaining capacity.
- Expiration.
- Validation errors.

License installation, replacement, and removal depend on the selected licensing provider and are not committed POC functions.

---

## 37. Capability Discovery

ARC shall expose installed and licensed capabilities for:

- Source adapters.
- Destination adapters.
- Codecs.
- Containers.
- Signal parsers.
- Signal generators.
- Local audio backends.
- Platform features.
- Device Server integration.
- License restrictions.

UI forms shall be constrained or generated using capability schemas.

Capabilities shall distinguish installed, enabled, licensed, unavailable, and unsupported states.

---

## 38. Configuration Import and Export

POC:

- Export one resource.
- Import one resource as a draft.
- Validate imported resource.
- Apply through normal revision and activation workflow.
- Exclude plaintext secret values.

Complete configuration import/export is included through the backup/restore capability rather than an independent ungoverned bulk endpoint.

Environment-specific overrides may be applied only through a documented schema and validation path.

---

## 39. Backup and Restore API

The POC shall support secure backup and restore of configuration and persistent operational state.

### 39.1 Backup contents

- Sources and revisions.
- Destinations and revisions.
- Desired routes.
- Presets.
- Signal mappings.
- Enabled states.
- System configuration.
- Authentication metadata.
- API-key metadata, without reusable plaintext key values.
- Secret store only when encrypted with deployment-approved protection.
- Persistent policies and retention settings.

### 39.2 Exclusions

- Runtime buffers.
- WebSocket sessions.
- Transient operations.
- In-flight media.
- Unencrypted secret values.
- Runtime statistics and queues unless explicitly persisted as historical data.

### 39.3 Operations

- Create backup.
- List backup metadata.
- Validate backup.
- Dry-run restore and impact report.
- Restore.
- Read operation status.
- Cancel before commit when safe.

Restore shall use an asynchronous operation and enter a controlled maintenance state for mutations.

---

## 40. Rate Limiting

Rate limits shall apply independently by identity and category:

- Authentication attempts.
- Configuration mutations.
- Route commands.
- Signal injection.
- Probe/test operations.
- Event subscriptions.
- Statistics subscriptions.
- Backup/restore.

Exceeding a limit returns `429 Too Many Requests` with retry guidance.

Rate-limit responses should provide applicable limit, remaining allowance, and reset/retry information without exposing other principals.

---

## 41. Request and Collection Limits

ARC shall define and publish limits for:

- JSON request size.
- Batch route count.
- Preset count.
- Tags per resource.
- String lengths.
- Imported configuration size.
- Backup size.
- Event-subscription count.
- Concurrent operations.
- Expansion depth.
- Page size.

Limits shall appear in API documentation and capability responses.

---

## 42. Time and Units

Public JSON timestamps shall use RFC 3339 UTC with explicit `Z` or offset. Local time without offset shall be rejected.

Media timestamps shall use separately documented clock units.

Field names shall contain units where applicable:

```json
{
  "latencyMs": 1000,
  "sampleRateHz": 48000,
  "bitrateKbps": 192,
  "durationMs": 250
}
```

---

## 43. Localization and Machine Stability

POC errors shall include:

- Stable machine-readable ARC code.
- English title and message.

Future localization may alter human-readable messages but shall not change machine codes or enum semantics.

---

## 44. SDK Generation

The POC shall generate:

- TypeScript client for the browser UI.
- C# client for automation integration, unless the automation implementation selects another required language.

SDKs shall be generated from OpenAPI rather than maintained as independent handwritten contracts.

Other SDKs may be generated later.

---

## 45. Browser and Integration API

The browser UI shall consume the same domain API used by integrations, optionally through a thin composition/backend-for-frontend layer.

Business rules shall not be duplicated in a UI-specific service.

The UI-specific layer may aggregate calls or tailor payloads but must invoke the same domain services and permission checks.

---

## 46. CORS

ARC shall support:

- Same-origin ARC hosting.
- Separate Nginx-hosted UI.
- UI hosted on another approved domain.

CORS shall use an explicit origin allowlist. Wildcard origins are prohibited for credentialed access.

Allowed methods, headers, credentials, and preflight caching shall be configurable within secure bounds.

---

## 47. TLS and Reverse Proxy

Primary deployment uses reverse-proxy TLS termination through Nginx or Nginx Proxy Manager. Optional direct TLS is supported.

ARC shall trust forwarded headers only from configured proxy addresses.

It shall validate and normalize forwarded protocol, host, and client-address information before using them for URLs, audit, or security decisions.

---

## 48. Internal Service Calls

Components sharing a process shall use in-process/domain interfaces.

Public HTTP API and Device Server adapter shall invoke the same domain services.

An internal RPC interface may be introduced for separately deployed components, but it shall preserve the same validation, permissions, revisions, and audit behavior.

---

## 49. Availability During Media Failures

Configuration, health, and routing APIs shall remain available when:

- A source fails.
- A destination fails.
- Device Server disconnects.
- A signal parser fails.
- Some media dependencies are degraded.

The API shall report those failures rather than becoming unavailable.

---

## 50. Shutdown Behavior

During controlled shutdown ARC shall:

- Stop accepting new mutations.
- Return a retryable maintenance problem response.
- Allow reads briefly when safe.
- Drain or cancel operations according to policy.
- Reject operations that cannot complete before shutdown.
- Close event streams with a shutdown reason and final sequence.
- Update readiness before process termination.

---

## 51. POC Performance Targets

Starting targets, subject to capacity testing:

- 25 concurrent API clients.
- 10 concurrent UI sessions.
- 25 WebSocket connections.
- 100 ordinary requests per second per instance.
- 20 route mutations per second.
- 16 concurrent long-running operations.
- Event replay bounded by configurable time and storage.
- Audit query ranges bounded by configurable date range and pagination.

Rate limits and resource admission may be set lower than absolute transport capacity to preserve media stability.

---

## 52. Endpoint Families

The OpenAPI contract shall include at minimum:

```text
/api/v1/sources
/api/v1/destinations
/api/v1/routes
/api/v1/route-presets
/api/v1/signal-mappings
/api/v1/signals
/api/v1/gpio
/api/v1/health
/api/v1/statistics
/api/v1/operations
/api/v1/events
/api/v1/licenses
/api/v1/audit
/api/v1/auth
/api/v1/api-keys
/api/v1/secrets
/api/v1/system
/api/v1/capabilities
/api/v1/backups
/api/v1/restores
/api/v1/bootstrap
```

Exact subresources shall follow the associated domain specifications.

---

## 53. Reference Examples

### 53.1 Asynchronous probe

```http
POST /api/v1/sources/source-1/operations/probe
Idempotency-Key: probe-source-1-001
If-Match: "revision-7"
X-Correlation-ID: corr-001
```

```json
{
  "operationId": "op-001",
  "type": "source-probe",
  "status": "queued",
  "cancellable": true,
  "correlationId": "corr-001"
}
```

### 53.2 Merge Patch

```http
PATCH /api/v1/destinations/dest-1
Content-Type: application/merge-patch+json
If-Match: "revision-10"
```

```json
{
  "processing": {
    "gainDb": -2.5
  }
}
```

### 53.3 Event subscription request

```json
{
  "action": "subscribe",
  "topics": ["routes", "health", "operations"],
  "statisticsIntervalMs": 1000,
  "lastGlobalSequence": 10400
}
```

---

## 54. Functional Acceptance Criteria

### 54.1 Contract and resources

- OpenAPI 3.1 validates in CI.
- Interactive docs and schemas are available.
- TypeScript and C# clients generate successfully.
- Resource endpoints use stable IDs and revision control.
- Unknown write fields are rejected.
- Draft/validated/active/rollback workflows function.

### 54.2 Authentication and permissions

- Basic and API-key POC authentication function.
- API keys are scoped, expirable, revocable, and displayed once.
- Permission checks apply to REST and event topics.
- Secret values are never returned.

### 54.3 Concurrency and idempotency

- ETag/If-Match behavior returns 412 correctly.
- Domain conflicts return 409.
- Retried idempotent operations do not duplicate effects.
- Idempotency-key payload mismatch is rejected.

### 54.4 Operations and errors

- Long actions return 202 and operation resources.
- Operation states and per-item results are accurate.
- Cancellation follows commit-point rules.
- Problem details are RFC 9457-compatible and contain extensions.
- Stack traces are absent.

### 54.5 Collections and snapshots

- Cursor pagination, filters, sorting, and expansion function.
- Bootstrap and domain snapshots contain revision markers.
- Request and collection limits are enforced and discoverable.

### 54.6 Events

- WebSocket is read-only.
- All required topics enforce scopes.
- Events have global sequence and event IDs.
- At-least-once delivery and deduplication operate.
- Replay works within retention.
- Snapshot recovery works outside retention.
- Statistics subscriptions honor intervals and limits.

### 54.7 Operational APIs

- Health, metrics, audit, license, and capability APIs function.
- Control API remains available during isolated media failures.
- CORS allowlisting and trusted-proxy behavior function.
- Shutdown behavior is deterministic.

### 54.8 Backup and restore

- Backup captures defined persistent state.
- Excluded transient state is absent.
- Secrets are not exported unencrypted.
- Restore supports validation, dry run, operation tracking, and controlled commit.

---

## 55. Required Test Scenarios

1. Validate OpenAPI document.
2. Generate TypeScript client.
3. Generate C# client.
4. Create disabled draft source.
5. Create disabled draft destination.
6. Activate and enable through explicit operations.
7. Update with PUT and valid If-Match.
8. Update with Merge Patch.
9. Reject unknown write field.
10. Return 412 for stale ETag.
11. Return 409 for domain conflict.
12. Create API key and display once.
13. Rotate and revoke API key.
14. Enforce every scope family.
15. Create/rotate/delete secret without reveal.
16. Generate correlation ID when absent.
17. Preserve supplied trace/correlation context.
18. Retry idempotent route operation.
19. Reject reused key with changed payload.
20. Start asynchronous source probe.
21. Inspect operation progress and result.
22. Cancel cancellable operation.
23. Reject cancellation after commit point.
24. Return 201, 202, and 204 appropriately.
25. Return problem details for validation failure.
26. Return field-level errors.
27. Confirm no stack trace disclosure.
28. Validate supplied unsaved object.
29. Dry-run route and activation.
30. Cursor-page audit and operation collections.
31. Filter and sort resource lists.
32. Expand documented relationships.
33. Load browser bootstrap snapshot.
34. Authenticate WebSocket.
35. Enforce event-topic scopes.
36. Receive globally sequenced events.
37. Deduplicate replayed event.
38. Recover events since sequence.
39. Fall back to full snapshot outside retention.
40. Subscribe to one-second statistics.
41. Enforce minimum statistics interval.
42. Verify public minimal liveness.
43. Protect detailed readiness.
44. Protect metrics endpoint.
45. Read and filter audit history.
46. Read effective license entitlements and usage.
47. Discover installed/licensed capabilities.
48. Generate UI form constraints from capability schema.
49. Export/import one resource as draft.
50. Create secure backup.
51. Validate and dry-run restore.
52. Restore persistent configuration/state.
53. Confirm excluded transient data is absent.
54. Confirm secret values are not unencrypted in backup.
55. Enforce rate limits and 429 retry guidance.
56. Enforce JSON and batch-size limits.
57. Reject timestamp without offset.
58. Verify unit-bearing fields.
59. Verify enum write rejection/read tolerance.
60. Verify null/omitted semantics.
61. Run UI through documented API.
62. Test same-origin and allowed cross-origin UI.
63. Reject untrusted proxy headers.
64. Continue API operation with failed source.
65. Continue API operation with failed destination.
66. Continue API operation with Device Server disconnected.
67. Enter shutdown maintenance behavior.
68. Close event stream with shutdown reason.
69. Meet starting concurrency and throughput targets.
70. Run fault-injection tests across domain dependencies.

---

## 56. Mandatory End-to-End Demonstrations

Because no subset was identified, all proposed demonstrations are mandatory:

1. Create, validate, activate, and operate a source entirely through the API.
2. Create, validate, activate, and operate a destination through the API.
3. Route and replace audio through the API.
4. Execute fan-out, batches, and presets.
5. Receive ordered real-time events.
6. Recover missed events through replay or snapshot.
7. Reject stale updates through revision protection.
8. Retry an idempotent mutation without duplicate execution.
9. Return standardized problem details.
10. Enforce permissions and locks.
11. Create and rotate a secret without returning its value.
12. Probe a source asynchronously and inspect operation status.
13. Cancel a cancellable operation.
14. Query health, capabilities, license usage, statistics, and audit history.
15. Control GPIO or signal generation through the API.
16. Demonstrate Device Server using the same domain rules.
17. Enforce rate and request-size limits.
18. Regenerate API clients from the OpenAPI document.
19. Run the browser UI using the documented API.
20. Continue API operation while an individual media pipeline is failed.

---

## 57. API Test Tooling

The POC shall include:

- OpenAPI validation.
- Contract tests.
- Generated mock server.
- Generated test client.
- Authentication tests.
- Permission tests.
- Idempotency tests.
- Concurrency tests.
- WebSocket replay tests.
- Rate-limit tests.
- Fault-injection tests.
- Performance tests.

Tests shall execute in CI and produce machine-readable reports.

---

## 58. Non-Functional Requirements

- API failure shall not unnecessarily terminate media pipelines.
- Individual media failure shall not make the API unavailable.
- API schemas, events, and errors shall be versioned.
- All mutations shall be authenticated, authorized, revision-aware, and auditable.
- Operations and queues shall be bounded.
- Responses shall avoid sensitive values.
- Event delivery shall be resumable within retention.
- Backward compatibility rules shall be documented.
- Capacity targets shall be tested on representative Intel and ARM deployments where applicable.

---

## 59. AI-Assisted Implementation Guidance

When used in a VS Code AI prompt, the coding assistant shall:

1. Treat this specification and OpenAPI as required contracts.
2. Build domain services independent of HTTP handlers.
3. Ensure UI, public API, and Device Server adapter call shared domain services.
4. Generate OpenAPI 3.1 and validate it in CI.
5. Generate TypeScript and C# clients from OpenAPI.
6. Implement uniform authentication, permissions, correlation, audit, and problem details middleware.
7. Implement ETag/If-Match and explicit revisions.
8. Reject unknown write fields.
9. Implement JSON Merge Patch correctly.
10. Implement idempotency storage by operation class.
11. Use operation resources for long-running work.
12. Make cancellation commit-point aware.
13. Implement cursor pagination and bounded filters/expansion.
14. Build ordered, read-only WebSocket events with replay and snapshot recovery.
15. Protect metrics, detailed health, and capability-sensitive information.
16. Never log, export, or return plaintext secrets.
17. Implement secure backup/restore with dry run and maintenance-state commit.
18. Add rate, request-size, collection, and subscription limits.
19. Include contract, security, concurrency, replay, fault, and performance tests.
20. Preserve compatibility with source, destination, routing, and signal specifications.

---

## 60. Documented Assumptions Requiring Later Review

- Backup/restore is interpreted as configuration plus persistent operational state, not runtime media state.
- Secret backup requires deployment-approved encryption; otherwise only secret references/metadata are backed up.
- API-key service integration is included in the POC.
- C# is the initial automation-side SDK assumption because no implementation language was specified.
- WebSocket event publication is globally sequenced and at-least-once within retained history.
- Audit export is post-POC.
- Full independent configuration import/export is represented through backup/restore for the POC.
- License installation remains provider-dependent.
- Direct TLS is optional; reverse proxy is primary.
- All twenty suggested API demonstrations are mandatory because no subset was identified.

---

## 61. Definition of Done

The ARC API and real-time control interface are complete for the POC when:

- All required endpoint families are implemented and documented.
- OpenAPI, schemas, interactive docs, examples, and generated clients are reproducible.
- Authentication, scopes, API keys, secrets, revisions, concurrency, idempotency, and audits function.
- Long-running operations and cancellation function.
- RFC 9457-compatible errors are consistent.
- Pagination, filtering, sorting, expansion, snapshots, and bootstrap function.
- WebSocket events, global sequence, replay, deduplication, and snapshot recovery function.
- Health, metrics, capabilities, licensing, audit, backup, and restore meet this specification.
- Browser UI and Device Server adapter use the same domain rules.
- API remains operational during isolated media failures.
- Rate, size, concurrency, and performance targets are tested.
- All required tests and mandatory demonstrations pass.
