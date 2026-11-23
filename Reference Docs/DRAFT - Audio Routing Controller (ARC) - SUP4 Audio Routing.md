# ARC Audio Routing and Crosspoint Control Specification

**Document status:** Draft for product and technical review  
**Application:** Audio Routing Controller (ARC)  
**Document role:** Jira attachment, engineering requirements, POC scope, and AI-assisted implementation specification  
**Related documents:** ARC Epic, ARC Audio Input Source Handling Specification, ARC Audio Output Destination Handling Specification, and ARC Signal Handling and WideOrbit Device Server Integration Specification

---

## 1. Purpose

This document defines how ARC shall create, validate, execute, observe, persist, replace, group, lock, audit, undo, and restore audio routes between configured sources and destinations.

ARC routing shall support:

- Any compatible source to any destination.
- One source feeding many destinations.
- Zero or one source assigned to each destination.
- Immediate and staged crosspoint workflows.
- Atomic replacement of an occupied destination.
- Best-effort fan-out.
- Atomic and best-effort multi-route operations.
- Saved route presets, also called salvos.
- Browser, API, automation, and Device Server control.
- Real-time synchronization across connected clients.
- Source-specific cue and metadata behavior on each route.

---

## 2. Product Decisions Summary

### 2.1 Required POC route operations

The POC shall support:

- Route a source to an empty destination.
- Explicitly replace an existing destination route.
- Remove a route.
- Route one source to several destinations.
- Save and execute route presets.
- Undo the most recent route change.
- Roll back a group of route changes.

Multi-route operations are also required by the detailed product decisions and API requirements. Scheduled routing and temporary preview routing are not POC functions. Scheduling remains the responsibility of the automation system.

### 2.2 Terminology

- API and configuration terminology: source, destination, and route.
- UI matrix terminology: crosspoint.
- Saved grouped route changes: Route Preset, with Salvo accepted as an alternate broadcast term.

### 2.3 Core cardinality

- One destination has zero or one desired source.
- One destination has zero or one active source.
- One source may feed any number of destinations.
- Mixing is not supported.
- Independent left/right routing is not supported in the POC.

### 2.4 Default workflows

- Immediate mode is the default crosspoint workflow.
- Take mode is configurable.
- Production replacement requires confirmation in immediate mode.
- Route replacement is atomic.
- Failed replacement leaves the existing route active.
- Fan-out is best effort by default.
- Atomic batch and preset execution are available when explicitly selected.

---

## 3. Routing Architecture

```text
Control origin
  UI | ARC API | Device Server | Automation | Preset | Recovery
                         |
                         v
                Permission and lock check
                         |
                         v
                 Route request validator
        endpoint | format | capacity | cue policy
                         |
                         v
                 Resource admission control
                         |
                         v
                    Route planner
       pass-through | transcode | delay | cue behavior
                         |
                         v
               Transaction/operation manager
                         |
                         v
        Source fan-out -> Destination media pipeline
                         |
                         v
        Route state, history, events, and telemetry
```

The routing control plane shall remain distinct from media pipelines. A control-plane failure shall not unnecessarily interrupt already active media flows.

---

## 4. Route Identity and Resource Model

A destination ID is the uniqueness key for a desired route because each destination accepts one source. Every route assignment or replacement shall also receive an immutable route-instance ID.

```yaml
route:
  routeInstanceId: generated-id
  destinationId: destination-id
  desiredSourceId: source-id
  activeSourceId: source-id-or-null
  state: active
  revision: 1042
  operationId: operation-id
  transition: hard-cut
  signalPolicy: {}
  metadataPolicy: {}
  createdAt: utc-time
  appliedAt: utc-time
  origin: ui
```

### 4.1 Desired and active state

ARC shall expose separately:

- Desired source.
- Active source.
- Desired route state.
- Observed active media state.

During preparation or failure, desired and active state may differ.

### 4.2 Route revision

Every accepted routing-state change shall advance a server-generated monotonically increasing routing revision.

Route-instance revision and global routing revision shall be separate values where needed for concurrency and event recovery.

---

## 5. Route-State Model

Routes shall support:

- Requested.
- Validating.
- Preparing.
- Switching.
- Active.
- Active but degraded.
- Failed.
- Removing.
- Removed.
- Unknown.

### 5.1 State meaning

- **Requested:** Accepted for processing but not yet validated.
- **Validating:** Endpoint, capability, policy, and resource checks are running.
- **Preparing:** Media path or destination processing is being prepared.
- **Switching:** Commit to the new route is in progress.
- **Active:** Desired and observed media flow agree and health is acceptable.
- **Active but degraded:** Route remains active but audio is health-policy muted, silent, or otherwise degraded.
- **Failed:** Desired assignment remains, but no acceptable active flow exists.
- **Removing:** Route teardown is in progress.
- **Removed:** No desired assignment remains.
- **Unknown:** ARC cannot reconcile desired and observed state.

### 5.2 Degraded source

If a source remains assigned but is health-policy muted, the route shall be Active but Degraded with an explicit mute/silence reason.

### 5.3 Failed destination

If a destination fails, the route shall be Failed while retaining the desired source assignment.

---

## 6. Immediate and Take Workflows

### 6.1 Immediate mode

Selecting an empty crosspoint executes the route after validation.

Selecting an occupied production destination shall show replacement confirmation before execution.

### 6.2 Take mode

Crosspoint selections shall enter Pending Take state. The operator may:

- Review staged changes.
- Dry run.
- Remove staged changes.
- Execute Take.
- Cancel the staged set.

### 6.3 Configurability

Immediate or Take mode shall be selectable by user session or control surface. Permissions may restrict use of immediate mode.

---

## 7. Atomic Route Replacement

Replacement shall be atomic from the operator and API perspective.

Required request fields:

- Destination ID.
- Replacement source ID.
- Expected current source ID or route revision.
- Transition mode.
- Idempotency key.
- Optional warning acknowledgment.

ARC shall:

1. Validate the requested source and destination.
2. Validate expected current state.
3. Check locks and permissions.
4. Reserve required resources.
5. Prepare the replacement path.
6. Preserve the existing route until preparation succeeds.
7. Commit the hard cut.
8. Preserve the destination encoder and transport session where compatible.
9. Record actual switch time and outcome.

If preparation fails, the existing route shall remain active.

---

## 8. Fan-Out Routing

ARC shall allow one source to route to multiple destinations through:

- Selecting one source and several destinations.
- Selecting several crosspoints in the grid.
- An API operation containing one source and many destination IDs.

Fan-out shall default to best-effort execution.

- Each destination shall receive an independent result.
- Failure of one destination shall not prevent successful routes to others.
- A caller may explicitly request atomic execution.
- Slow or failed destinations shall never block source processing or sibling destinations.

---

## 9. Multi-Route Operations

A route operation may contain several independent assignments and removals.

The operation shall support:

- Validate all before execution.
- Atomic all-or-nothing mode.
- Best-effort mode.
- Per-route results.
- Rollback on atomic failure.
- Explicit rollback after best-effort completion.

### 9.1 Operation states

- Requested.
- Validating.
- Reserving.
- Preparing.
- Committing.
- Completed.
- Partially completed.
- Rolling back.
- Rolled back.
- Failed.
- Timed out.

---

## 10. Routing Transactions

Atomic operations shall:

1. Validate every change.
2. Validate expected current state.
3. Check permissions and locks.
4. Reserve resources.
5. Prepare media paths.
6. Commit all changes.
7. Roll back if commit cannot complete.

Transaction timeout shall be configurable per operation within deployment-enforced minimum and maximum limits.

No transaction may hold resources indefinitely.

---

## 11. Best-Effort Operations

For best-effort fan-out or batch operations:

- Successful changes remain active.
- Failed changes report individual errors.
- Operation state becomes Partially completed when appropriate.
- ARC shall not automatically undo successful changes.
- An explicit rollback operation may reverse completed items using the operation record.

---

## 12. Route Presets and Salvos

The primary user-facing term shall be **Route Preset**. The API and documentation may accept **salvo** as an alias.

A preset may contain:

- Route assignments.
- Route removals.
- Transition policy.
- Expected current-state conditions.
- Execution order where required.
- Description.
- Tags.

Destination processing and cue/metadata settings shall remain in normal configuration for the POC, except that standard route-level signal overrides may be referenced.

### 12.1 Preset operations

- Create.
- Read.
- Update.
- Delete.
- Clone.
- Validate.
- Dry run.
- Execute atomically.
- Execute best effort.
- Roll back execution.
- Activate through API.

Preset enable/disable is not required.

### 12.2 Strict and force modes

- Strict is the default.
- Strict validates expected current-state conditions.
- Force ignores preset expected-state conflicts but still obeys permissions, locks, compatibility, license, and resource admission.
- Force requires explicit permission and audit.

### 12.3 Scale

POC defaults, configurable by deployment:

- 100 saved presets.
- 64 destination changes per preset.
- 64 changes per API batch.

---

## 13. Scheduling

ARC shall not provide a POC scheduler for route execution.

The automation system shall schedule calls to immediate, batch, or preset-execution APIs. The API shall be deterministic, idempotent, and suitable for external scheduling.

---

## 14. Route Validation

Before execution, ARC shall validate:

- Source exists, is enabled, and is not deleted.
- Destination exists, is enabled, and is not deleted.
- Endpoint operational availability.
- Codec compatibility.
- Pass-through eligibility.
- Available transcode path.
- Channel compatibility.
- Sample-rate conversion.
- Destination capacity.
- License state.
- Delay-buffer capacity.
- Cue compatibility.
- Metadata compatibility.
- Estimated processing cost.
- Lock state.
- Caller permission.
- Expected current route.

Desired routes may be created for degraded, stopped, waiting, or failed endpoints when they are otherwise enabled and compatible. The route shall show why it cannot yet become active.

### 14.1 Warnings

UI warnings require confirmation. API warnings shall be machine-readable and require an explicit acknowledgment field when the policy requires confirmation.

---

## 15. Route Planning and Cost

Before execution ARC shall report:

- Pass-through or transcode.
- Required codec conversion.
- Channel conversion.
- Expected output codec.
- Estimated additional latency.
- Destination delay.
- Cue offset.
- Estimated CPU or normalized processing cost.
- Buffer/delay memory.
- Potential quality loss.
- Effective cue and metadata outcome.

The grid shall support compatible-only filtering.

---

## 16. Resource Admission Control

A new route shall be rejected by default when it exceeds safe host capacity.

Applicable resources:

- CPU.
- Memory.
- Encoder slots or equivalent capacity.
- Network bandwidth.
- Buffer and delay memory.
- Preview sessions.
- License limits.

A deployment may allow an explicit administrative override. Overrides shall be permission-protected, audited, and return predicted risk.

---

## 17. Crosspoint Grid

Default layout:

- Sources as rows.
- Destinations as columns.

Orientation is fixed to this default for the POC because it was explicitly selected. A future implementation may add user-configurable orientation.

### 17.1 Grid functions

- Search.
- Tags.
- Environment filters.
- Role filters.
- Protocol filters.
- Health filters.
- Routed/unrouted filters.
- Compatible-only view.
- Favorites.
- Frozen headers.
- Virtualization.

The POC shall support a 64 by 64 grid containing 4,096 cells in one virtualized view.

### 17.2 Client state

The client shall maintain accurate state for the complete matrix. Rendering and high-frequency visual updates may be limited to visible cells.

### 17.3 Cell states

- No route.
- Active route.
- Desired but preparing.
- Pending Take.
- Degraded route.
- Failed route.
- Incompatible.
- Disabled endpoint.
- Locked.
- Selected for batch operation.
- Changed by another user.

Color shall never be the sole indicator. Icons, labels, patterns, or accessible text shall supplement color.

---

## 18. Route Detail View

Selecting a crosspoint shall show:

- Source and destination.
- Desired source.
- Active source.
- Route and operation state.
- Pass-through/transcode path.
- Effective codec.
- Estimated latency.
- Destination delay.
- Cue and metadata policy.
- Health.
- Last changed by.
- Last request and applied times.
- Route revision.
- Recent errors.
- Lock status.
- Resource estimate.

---

## 19. Real-Time Multi-User Synchronization

Route changes shall appear in all connected UI sessions without refresh.

ARC shall publish ordered route events containing the global routing revision.

A client that misses events may:

- Request changes since its last revision.
- Reload the full routing snapshot.

If incremental recovery cannot satisfy the request because history has expired, the server shall require a full snapshot reload.

---

## 20. Concurrent Edits

First valid operation wins. A stale operation shall fail with a conflict response containing:

- Current desired source.
- Current active source.
- Current route revision.
- Current global routing revision.
- Last change metadata where permitted.

Last-write-wins behavior is prohibited for route replacement.

---

## 21. Route Locks

ARC shall support:

- Administrative locks.
- Temporary operator locks.

A lock shall include:

- Lock ID.
- Destination ID.
- Owner.
- Reason.
- Creation time.
- Optional expiration.
- Allowed override role.
- Whether automation override is permitted.
- Revision.

### 21.1 Override

Automation or Device Server may bypass a lock only if:

- The lock permits automation override.
- The caller has the elevated override permission.

Both conditions are required. Every override shall be audited.

---

## 22. Permissions

Distinct permissions shall include:

- View routing.
- Make route.
- Replace route.
- Remove route.
- Execute batch operation.
- Execute preset.
- Create/edit preset.
- Lock/unlock destination.
- Override lock.
- Roll back operation.
- View audit history.
- Administrative capacity override.
- Force preset execution.

The POC may use one administrator identity with all permissions, but the internal permission model shall exist.

---

## 23. Route Origin and Precedence

Valid origins:

- Browser UI.
- ARC API.
- Device Server.
- Automation system.
- Preset execution.
- Recovery/failover process.
- Future scheduler.

No origin has inherent priority. Permissions, locks, expected state, admission control, and transaction ordering decide whether a request succeeds.

---

## 24. Routing API

The API shall expose:

- Full routing snapshot.
- Routes by source.
- Route by destination.
- Create route.
- Replace route.
- Remove route.
- Validate route.
- Dry-run route.
- Fan-out route.
- Batch route.
- Execute preset.
- Undo operation.
- Roll back operation.
- Read operation status.
- Read route history.
- Subscribe to route events.
- Lock/unlock destination.

### 24.1 Idempotency

Create, replacement, removal, fan-out, batch, preset, undo, and rollback requests shall accept idempotency keys.

A repeated identical request with the same key shall return the original operation result. Reuse of a key with a different payload shall be rejected.

### 24.2 Concurrency

Mutating operations shall include expected route revision, expected source, global routing revision, or another appropriate precondition.

---

## 25. Device Server Routing Exposure

ARC shall expose audio crosspoints through the Device Server plugin during the POC using the applicable switch-plugin capabilities.

The adapter shall:

- Enumerate source and destination counts or mappings.
- Translate Device Server crosspoint requests to ARC route operations.
- Publish confirmed route changes.
- Preserve one-source-per-destination rules.
- Enforce locks and permissions.
- Return errors for incompatible, stale, or unauthorized requests.
- Avoid treating a submitted request as active until ARC confirms the route.

Device Server routing and GPIO handling shall remain separate capabilities even when they share one plugin.

---

## 26. Route-Level Cue and Metadata Policy

Each route may override source and destination signal defaults.

Supported route overrides:

- Pass cue.
- Regenerate cue.
- Translate cue.
- Suppress cue.
- Cue offset.
- Forward metadata.
- Suppress metadata.

Precedence:

```text
Route override
    -> destination default
        -> source default
            -> application default
```

Route-specific audio delay is not part of the POC. ARC shall use destination audio delay and route-level cue offset.

---

## 27. Endpoint State Behavior

### 27.1 Source disabled

Preserve desired routes and apply each destination's source-failure/no-source behavior. Routes become degraded or failed as appropriate.

### 27.2 Destination disabled

Preserve desired route, stop the destination pipeline, and show the route as desired but inactive/failed.

### 27.3 Expected restart

During a source or destination pipeline restart:

- Preserve desired route.
- Show Preparing or Active but Degraded.
- Restore active flow automatically.
- Avoid creating a new route-instance ID solely because an endpoint restarted.

### 27.4 Endpoint deletion

Deletion of a routed endpoint requires explicit confirmation. ARC shall remove active assignments through an audited transaction and retain route history.

Soft-delete implementation is permitted, but deleted resources shall not be routable.

---

## 28. Endpoint Revision Changes

Before applying a source or destination revision that affects active routes, ARC shall recalculate every impacted route.

Order:

1. Preserve pass-through if compatible and policy permits.
2. Attempt automatic conversion under effective destination policy.
3. Reject applying the endpoint revision if an active route cannot remain valid.

The UI/API shall return impacted routes and reasons before apply.

---

## 29. Route Persistence and Recovery

Desired routes shall survive:

- ARC restart.
- Host restart.
- Source restart.
- Destination restart.
- Compatible endpoint revision changes.

After ARC starts:

1. Load desired routes.
2. Validate endpoint existence and active configuration.
3. Validate locks, licenses, and compatibility.
4. Restore enabled routes.
5. Reconcile desired and observed state.
6. Publish state transitions.

Invalid persisted routes shall remain as failed desired routes. ARC shall apply destination no-source behavior and require operator correction. It shall not silently delete them.

---

## 30. Undo and Rollback

ARC shall support:

- Destination-specific undo to the immediately previous desired assignment.
- Operation-level undo that reverses the most recent eligible routing operation.
- Rollback of preset or batch operations.

Undo shall validate current state and shall not silently overwrite newer unrelated changes.

If current state no longer satisfies undo preconditions, ARC shall return a conflict and offer a dry-run result for an explicit corrective operation.

---

## 31. Route History and Audit

Retention shall support both time and storage limits.

History shall include:

- Requester.
- Origin.
- Source and destination.
- Previous desired/active route.
- New desired/active route.
- Request time.
- Applied time.
- Result.
- Warnings/errors.
- Correlation ID.
- Idempotency key reference.
- Route-instance ID.
- Route revision.
- Global routing revision.
- Lock or override information.
- Operation/preset ID.

---

## 32. Dry Run

Dry run shall return:

- Compatibility result.
- Required transcodes.
- Expected output codec.
- Estimated latency.
- Estimated resource effect.
- Locks and permissions.
- License result.
- Current-state conflicts.
- Cue and metadata outcome.
- Exact routes that would change.
- Expected no-source or failure behavior.

The UI shall automatically offer dry run before batch and preset execution.

Dry run shall not reserve resources or alter route state.

---

## 33. Routing Availability

Desired routes may be created when:

- Source is degraded.
- Destination is waiting for peer.
- Destination is failed.
- Source is stopped.
- Destination is stopped.

Desired routes shall not be created when an endpoint is:

- Disabled.
- Deleted.
- Structurally incompatible with no permitted conversion.
- Blocked by license state in a way that prohibits activation.

The response shall clarify whether the route is desired-only or active.

---

## 34. Observability

ARC shall expose metrics for:

- Active routes.
- Desired but inactive routes.
- Degraded routes.
- Failed routes.
- Route changes by origin.
- Route-change latency.
- Validation failures.
- Transcode routes.
- Route conflicts.
- Lock conflicts and overrides.
- Batch results.
- Preset results.
- Rollbacks and undo.
- WebSocket clients.
- Missed-event incremental recovery.
- Full-snapshot recovery.
- Atomic transaction rollback.
- Switch latency.

Distributed traces shall correlate control request, validation, media preparation, commit, and route-state publication.

---

## 35. Routing Test Harness

The POC shall include a routing test harness able to:

- Create synthetic sources and destinations.
- Exercise a 64 by 64 matrix.
- Perform fan-out.
- Perform atomic and best-effort batches.
- Execute presets.
- Inject stale-revision conflicts.
- Exercise locks and overrides.
- Simulate source and destination failures.
- Verify persistence and recovery.
- Verify event ordering and revision recovery.
- Verify cue-policy precedence.
- Measure switch latency.
- Verify one failed destination does not affect siblings.
- Generate machine-readable results.

---

## 36. Reference API Examples

These examples illustrate behavior and are not the final API contract.

### 36.1 Create route

```json
{
  "destinationId": "dest-001",
  "sourceId": "source-001",
  "expectedRouteRevision": 17,
  "mode": "immediate",
  "idempotencyKey": "route-request-1001"
}
```

### 36.2 Replace route

```json
{
  "destinationId": "dest-001",
  "expectedCurrentSourceId": "source-001",
  "replacementSourceId": "source-002",
  "transition": "hard-cut",
  "warningAcknowledged": true,
  "idempotencyKey": "replace-request-1002"
}
```

### 36.3 Best-effort fan-out

```json
{
  "mode": "best-effort",
  "sourceId": "source-001",
  "destinationIds": ["dest-001", "dest-002", "dest-003"],
  "idempotencyKey": "fanout-request-1003"
}
```

### 36.4 Atomic batch

```json
{
  "mode": "atomic",
  "timeoutMs": 10000,
  "changes": [
    {"sourceId": "source-a", "destinationId": "dest-1"},
    {"sourceId": "source-b", "destinationId": "dest-2"},
    {"remove": true, "destinationId": "dest-3"}
  ],
  "idempotencyKey": "batch-request-1004"
}
```

---

## 37. Functional Acceptance Criteria

### 37.1 Core routing

- Route a source to an empty destination.
- Reject ordinary assignment to an occupied destination.
- Explicitly replace an occupied destination atomically.
- Keep the existing route when replacement preparation fails.
- Remove a route and apply destination no-source behavior.
- Fan one source to several destinations.
- Maintain one desired and one active source maximum per destination.

### 37.2 Workflows

- Immediate and Take modes operate.
- Production replacement confirmation operates.
- Pending Take selections are visible and cancellable.
- Batch and preset dry-run results are available.
- Undo and rollback validate current state.

### 37.3 Transactions and presets

- Atomic batch validates, reserves, prepares, commits, and rolls back.
- Best-effort batch returns per-route results and preserves successes.
- Route Presets support strict and force modes.
- Presets can be cloned, validated, dry-run, executed, and rolled back.
- Selected scale limits are enforced and configurable.

### 37.4 Grid and real time

- Full 64 by 64 virtualized grid renders and remains responsive.
- All cell states are accessible without color dependence.
- Route changes appear in all connected sessions.
- Incremental recovery and full snapshot recovery work.
- First valid concurrent operation wins and stale requests return conflict.

### 37.5 Locks and permissions

- Administrative and temporary locks operate.
- Lock metadata and expiration operate.
- Automation override requires both lock permission and elevated caller permission.
- Distinct routing permissions exist internally.

### 37.6 Validation and admission

- Every required compatibility and resource check executes.
- UI warnings require confirmation.
- API warnings are structured.
- Unsafe routes are rejected by default.
- Route cost and latency information are available.
- Compatible-only grid filtering works.

### 37.7 Persistence and endpoint state

- Desired routes survive application and host restart.
- Routes survive endpoint restart and recover automatically.
- Invalid persisted routes remain failed and visible.
- Disabling endpoints preserves desired assignment as specified.
- Deletion is explicit and audited.
- Incompatible endpoint revisions are rejected before disrupting routes.

### 37.8 Signals and Device Server

- Route policy precedence resolves correctly.
- Different fan-out routes may use different cue/metadata policies.
- Device Server crosspoint control creates confirmed ARC route operations.
- Device Server requests obey locks, permissions, compatibility, and stale-state checks.

---

## 38. Required Test Scenarios

1. Route source to empty destination.
2. Reject ordinary route to occupied destination.
3. Replace occupied destination atomically.
4. Fail replacement preparation and preserve existing route.
5. Remove route and send destination silence.
6. Fan out one source through UI selection.
7. Fan out through API.
8. Execute best-effort fan-out with one failure.
9. Execute atomic fan-out with one failure and roll back.
10. Execute multi-source atomic batch.
11. Execute best-effort batch and inspect per-route results.
12. Explicitly roll back successful best-effort items.
13. Stage and Take a route.
14. Cancel Pending Take changes.
15. Confirm production replacement in immediate mode.
16. Create, validate, dry-run, clone, and execute a preset.
17. Reject a strict preset state conflict.
18. Execute force preset with permission.
19. Undo one destination route.
20. Undo an operation.
21. Reject undo after incompatible newer change.
22. Persist routes through ARC restart.
23. Persist routes through host restart.
24. Preserve route through source restart.
25. Preserve route through destination restart.
26. Preserve invalid persisted route as failed.
27. Route to degraded source as desired-only/degraded.
28. Route to waiting-for-peer destination.
29. Reject disabled endpoint route.
30. Validate pass-through route.
31. Validate automatic transcode route.
32. Reject route with no conversion path.
33. Reject route exceeding default capacity policy.
34. Exercise authorized capacity override.
35. Display route cost, latency, delay, and cue offset.
36. Filter compatible-only grid.
37. Exercise complete 64 by 64 grid.
38. Verify non-color cell indicators.
39. Synchronize several browser sessions.
40. Recover a client using changes-since revision.
41. Recover a client using full snapshot.
42. Reject stale concurrent replacement.
43. Create administrative lock.
44. Create expiring operator lock.
45. Reject unauthorized lock override.
46. Permit automation override only under both required conditions.
47. Control a route through ARC API.
48. Retry idempotent request and return original result.
49. Reject idempotency-key payload mismatch.
50. Control route through Device Server.
51. Verify Device Server waits for ARC confirmation.
52. Apply route-specific cue override.
53. Apply metadata suppression on one fan-out route only.
54. Verify route override precedence.
55. Preserve desired route when source is disabled.
56. Preserve inactive desired route when destination is disabled.
57. Delete routed endpoint with confirmed audited transaction.
58. Reject incompatible endpoint revision apply.
59. Leave successful best-effort routes active after partial failure.
60. Verify one failed destination leaves siblings running.
61. Measure route switch latency.
62. Verify route history contents and retention enforcement.
63. Trace a route request through commit and publication.
64. Enforce preset and batch size limits.
65. Timeout an atomic transaction and release resources.

---

## 39. Mandatory End-to-End Demonstrations

Because no subset was identified, all proposed demonstrations are mandatory POC acceptance targets:

1. Route one source to one destination.
2. Fan one source out to several destinations.
3. Explicitly replace an occupied destination.
4. Preserve the output session during a compatible replacement.
5. Remove a route and apply destination no-source behavior.
6. Execute an atomic batch affecting several destinations.
7. Execute a best-effort batch with one intentional failure.
8. Execute and roll back a route preset.
9. Show a route update in several browser sessions in real time.
10. Reject a stale concurrent update.
11. Preserve and restore routes through ARC restart.
12. Preserve desired route through source and destination restart.
13. Prevent a locked destination from unauthorized change.
14. Route with automatic transcode after compatibility validation.
15. Resolve route-level cue and metadata policy.
16. Demonstrate that one failed destination does not affect other destinations.
17. Exercise the full virtualized 64 by 64 crosspoint view.
18. Control a route through the API.
19. Control a route through Device Server.
20. Audit and undo a routing operation.

---

## 40. Non-Functional Requirements

- No route operation shall require an ARC application restart.
- Control-plane failures shall not unnecessarily terminate active media flows.
- Routing operations shall be deterministic, revisioned, and auditable.
- Transactions, reservations, queues, and event history shall be bounded.
- Slow or failed destinations shall not block sources or sibling destinations.
- All route events shall be ordered by global routing revision.
- API and UI shall remain responsive at 64 sources and 64 destinations.
- Permission and lock checks shall apply consistently to UI, API, automation, and Device Server.
- Resource admission shall use current measured capacity and deployment policy.
- Secrets shall not be part of route payloads or route history.

---

## 41. AI-Assisted Implementation Guidance

When used in a VS Code AI prompt, the coding assistant shall:

1. Treat this document as required behavior.
2. Implement routing as a control-plane service separate from media pipelines.
3. Use destination ID as route uniqueness key and generate route-instance IDs.
4. Preserve desired and active route states separately.
5. Implement global monotonically increasing routing revision.
6. Use optimistic concurrency and reject stale operations.
7. Require explicit replacement for occupied destinations.
8. Preserve the current route until replacement preparation succeeds.
9. Implement best-effort fan-out by default and explicit atomic mode.
10. Build transaction phases for validation, reservation, preparation, commit, and rollback.
11. Keep every transaction and operation bounded and cancellable.
12. Implement Route Presets with strict/force modes and dry run.
13. Protect destinations with administrative and temporary locks.
14. Apply the internal permission model even with one POC administrator.
15. Make route API requests idempotent.
16. Build WebSocket or equivalent ordered real-time events plus snapshot recovery.
17. Virtualize the 64 by 64 matrix and preserve full client state.
18. Apply route cue/metadata override precedence exactly as specified.
19. Integrate Device Server crosspoint control without bypassing route validation.
20. Persist desired routes and reconcile them at startup.
21. Include synthetic routing harnesses and conflict/failure injection.
22. Generate unit, integration, concurrency, transaction, performance, persistence, and recovery tests.
23. Instrument route validation, switch latency, failures, conflicts, locks, and rollbacks.
24. Never infer that a requested route is active until media state confirms it.
25. Maintain compatibility with source, destination, and signal specifications.

---

## 42. Documented Assumptions Requiring Later Review

- Multi-route operations are included despite not being starred because later selections require atomic and best-effort batches.
- Fan-out defaults to best effort because that option was explicitly selected.
- Crosspoint orientation is sources as rows and destinations as columns; user-configurable orientation is deferred.
- Scheduled routing remains in the automation system.
- Route-specific audio delay is excluded from the POC.
- Device Server audio crosspoint control is included in the POC.
- All twenty suggested demonstrations are mandatory because no subset was identified.
- One hundred presets and 64 changes per preset/batch are configurable POC defaults.
- Routing history retention values remain deployment-configurable.
- Transaction timeout values remain configurable within deployment limits.
- Capacity override exists only when deployment policy permits it.

---

## 43. Definition of Done

Audio routing and crosspoint control are complete for the POC when:

- All required route operations, presets, undo, and rollback function.
- One-source-per-destination and unlimited fan-out semantics are enforced.
- Desired and active state remain accurate through transitions and failures.
- Immediate and Take workflows operate.
- Atomic replacement preserves the old route on preparation failure.
- Atomic and best-effort batch behavior is deterministic.
- Locks, permissions, idempotency, conflicts, and expected-state checks work across every control origin.
- Route validation, admission control, transcode planning, and cue-policy resolution work.
- Full 64 by 64 grid and real-time synchronization satisfy the acceptance tests.
- Desired routes persist and reconcile after ARC, host, source, and destination restarts.
- Device Server audio crosspoint control is integrated.
- Audit, history, metrics, traces, synthetic test harness, and all mandatory demonstrations are complete.
