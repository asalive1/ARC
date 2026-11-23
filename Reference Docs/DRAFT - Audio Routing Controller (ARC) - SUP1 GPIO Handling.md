# ARC Signal Handling and WideOrbit Device Server Integration Specification

**Document status:** Draft for review  
**Application:** Audio Routing Controller (ARC)  
**Primary audience:** Product Management, Architecture, Development, QA, and POC implementation teams  
**Intended use:** Jira attachment, engineering requirements, and AI-assisted implementation prompt  

---

## 1. Purpose

This document defines how the Audio Routing Controller (ARC) shall receive, identify, decode, normalize, route, translate, generate, and monitor control signals associated with audio streams. It also defines how ARC shall communicate normalized control events to the WideOrbit Device Server so those events can be exposed to the automation system as logical GPI inputs or accepted from the automation system as logical GPO requests.

ARC is both:

1. An audio routing application.
2. A timed signaling gateway between media transports, external command formats, the WideOrbit Device Server, and outgoing media streams.

The objective is to preserve the timing and meaning of control signals while avoiding a design that treats every transported command as a simple electrical relay closure.

---

## 2. Scope

This specification covers:

- Control signals embedded in or associated with incoming audio streams.
- SCTE-35, ASCII, XML, timed metadata, private data, and vendor-specific commands.
- Normalization of different signal formats into a common ARC event model.
- Translation of normalized events into logical Device Server GPI state changes.
- Translation of Device Server GPO commands into generated output signals.
- Pass-through, suppression, translation, regeneration, and fan-out of signals.
- Timing alignment between audio and control events.
- Signal mapping, validation, deduplication, debounce, scheduling, monitoring, and audit history.
- Device Server plugin responsibilities.
- POC implementation expectations and acceptance criteria.

This specification does not define the internal scheduling behavior of the WideOrbit automation system after Device Server publishes a logical GPI event.

---

## 3. Terminology

### 3.1 GPI

A General Purpose Input presented to the automation system. Within ARC and Device Server, a GPI is a logical input state or pulse. It does not necessarily represent a physical electrical input.

### 3.2 GPO

A General Purpose Output requested by the automation system. Within ARC and Device Server, a GPO is a logical output command. ARC may translate that command into an in-stream cue, a sidecar message, an API action, or another supported signal.

### 3.3 Control event

A normalized ARC representation of a received, generated, or scheduled signaling occurrence. A control event may contain timing, duration, identity, state, and format-specific metadata.

### 3.4 State

A logical ON or OFF condition that remains in effect until changed.

### 3.5 Pulse

A temporary logical state transition with a defined duration and retrigger policy.

### 3.6 Semantic event

An occurrence such as `break_start`, `break_end`, `program_start`, or `station_id`. A semantic event is not inherently attached to a Device Server pin.

### 3.7 Embedded signal

Control information carried within the same media container, manifest, or transport session as the associated audio.

### 3.8 Sidecar signal

Control information delivered through a separate connection, such as TCP, UDP, HTTP, or WebSocket, while remaining logically associated with an audio source or destination.

### 3.9 Effective time

The point on the media timeline when a control event is intended to take effect.

### 3.10 Dispatch time

The time at which ARC sends or publishes the translated action to Device Server or an output signal generator.

---

## 4. Design Principles

ARC shall follow these principles:

1. **Separate audio transport from signal format.** SRT, RTP, and UDP describe transport behavior; they do not by themselves define the embedded command format.
2. **Normalize before execution.** Incoming commands shall be decoded into a common event model before being mapped to Device Server operations.
3. **Preserve semantic information.** SCTE-35 and other structured events shall retain timing, duration, event identity, cancel status, and relevant descriptors until mapping or output processing is complete.
4. **Separate events from pin mappings.** Parsers shall not contain Device Server pin assignments.
5. **Use logical states at the Device Server boundary.** Electrical polarity and vendor-specific representation shall remain within the relevant adapter.
6. **Distinguish requested and observed state.** Successful command transmission shall not automatically be treated as confirmed execution.
7. **Bind timed events to media clocks.** Audio pipeline latency, buffering, transcoding, and output timestamp changes shall be considered when scheduling signals.
8. **Make behavior configurable.** Timing, retrigger, duplicate, reconnect, failover, and late-event policies shall be defined per source, mapping, or destination.
9. **Provide traceability.** Every accepted or rejected control event shall be traceable from input through mapping to final action.
10. **Design for plugins.** Format-specific and vendor-specific behavior shall be extensible without changing the core event-routing model.

---

## 5. High-Level Architecture

```text
Incoming Source
    |
    +-- Transport Receiver
    |       UDP | RTP | SRT | HLS | Icecast | AES67 | Local Device
    |
    +-- Container / Manifest Inspector
    |       MPEG-TS | HLS Manifest | Elementary Audio | Vendor Container
    |
    +-- Audio Path ------------------------------------+
    |       Demux -> Decode/Pass-through -> Route      |
    |                                                   +--> Output Pipeline
    +-- Signal Extraction                              |
            SCTE-35 | Timed Metadata | ASCII | XML     |
            Private PID | Sidecar | Vendor Plugin      |
                    |                                  |
                    v                                  |
            Parser and Validator                       |
                    |                                  |
                    v                                  |
            ARC Normalized Control Event Bus           |
            |            |             |               |
            |            |             |               |
            v            v             v               v
      Mapping Engine  Audit Store  Live Monitor  Signal Generator
            |                                          |
            v                                          v
    Device Server Adapter                       Output Injector
            |                                          |
            v                                          v
    WideOrbit Device Server                SRT/RTP/UDP/HLS/etc.
            |
            v
    WideOrbit Automation for Radio
```

### 5.1 Major components

ARC shall contain the following logical components:

- Transport receiver and sender.
- Container and manifest inspector.
- Audio processor and routing engine.
- Signal extractor.
- Format-specific parser and validator.
- Normalized control event bus.
- Mapping and policy engine.
- Event scheduler.
- Device Server communication service.
- Native Device Server plugin.
- Output signal generator and mux/injector.
- Configuration API.
- Real-time event API or WebSocket interface.
- Audit and diagnostics subsystem.
- OpenTelemetry instrumentation.

Components may operate within one process for the POC, but their interfaces shall remain logically separated.

---

## 6. Source Signal Configuration

Each ARC source shall independently define its audio and signaling characteristics.

### 6.1 Required source fields

- Source ID.
- Friendly name.
- Enabled state.
- Network interface.
- Transport type.
- Connection mode and addressing fields required by the selected transport.
- Payload or container type.
- Audio program, track, or PID selection where applicable.
- Signaling mode.
- Signaling program, PID, manifest tag, channel, or sidecar endpoint where applicable.
- Automatic detection enabled or disabled.
- Clock and timestamp source.
- Signal timing policy.
- Signal handling policy.
- Mapping profile.
- Duplicate handling policy.
- Late-event policy.
- Reconnect and failover policy.

### 6.2 Supported signaling modes

The design shall support the following extensible set:

- None.
- Automatic detection.
- MPEG-TS SCTE-35 PID.
- HLS SCTE-35 or DATERANGE signaling.
- Timed ID3 or other timed metadata.
- Private MPEG-TS data PID.
- Vendor ASCII.
- Vendor XML.
- Sidecar TCP.
- Sidecar UDP.
- HTTP or API event source.
- Custom parser plugin.

The initial POC may implement a subset, but unsupported selections shall be identified clearly by capability reporting rather than silently ignored.

### 6.3 Source signal handling policy

Each source shall support one or more of these policies:

- Consume locally.
- Pass unchanged.
- Validate and pass unchanged.
- Translate for Device Server.
- Translate for one or more output destinations.
- Parse and regenerate.
- Suppress.
- Audit only.

---

## 7. Signal Extraction

### 7.1 General behavior

ARC shall inspect the configured transport and container before attempting to interpret control data. Signal extraction shall not depend on decoding the audio when the signaling information is available independently in the container or manifest.

The extractor shall:

1. Identify the selected program, PID, manifest tag, metadata channel, or sidecar association.
2. Preserve the raw signal payload.
3. Capture arrival time and available media timestamps.
4. Forward the raw payload and timing context to the selected parser.
5. Report malformed, missing, unexpected, or unsupported signal data.
6. Continue handling audio according to source policy when signal parsing fails, unless the configured fail-closed policy requires otherwise.

### 7.2 MPEG transport stream sources

For an MPEG-TS source, ARC shall support independent discovery and selection of:

- Program number.
- PMT.
- Audio PID.
- Signaling or private-data PID.
- SCTE-35 PID where provided.
- PCR and PTS timing context.

ARC shall not assume that every program in a multi-program transport stream uses the same cue PID.

### 7.3 HLS sources

For HLS, ARC shall inspect both:

- Manifest-level signaling.
- In-band signaling within media segments when present and supported.

ARC shall correlate manifest events and in-band events to avoid duplicate execution when the same event is represented in more than one location.

### 7.4 Sidecar sources

A sidecar configuration shall explicitly associate a command connection with an ARC audio source or source group. The association shall remain stable across reconnects.

---

## 8. Parser and Validation Requirements

### 8.1 Common parser contract

Every parser shall accept:

- Raw payload.
- Source identity.
- Arrival timestamp.
- Media timestamp context when available.
- Parser configuration.

Every parser shall return either:

- One or more normalized control events, or
- A structured rejection with an error code, diagnostic message, and raw-payload reference.

A parser shall not directly execute a Device Server action.

### 8.2 SCTE-35 parsing

The SCTE-35 parser shall preserve, when present:

- Splice command type.
- Event identifier.
- Cancel indicator.
- Immediate or scheduled behavior.
- PTS and adjustment information.
- Break duration.
- Program or component scope.
- Segmentation descriptor values.
- Segmentation event identifier.
- Segmentation type.
- UPID information.
- Tier and encryption indicators.
- CRC validation result.
- Unknown descriptors.
- Original binary payload.

The parser shall support configurable behavior for:

- Invalid CRC.
- Encrypted or unsupported messages.
- Unknown command or descriptor types.
- Duplicate event identifiers.
- Events with timestamps outside the accepted scheduling window.
- Cancel messages without a matching pending event.

### 8.3 ASCII parsing

ASCII parser profiles shall define:

- Transport or data-channel association.
- Character encoding.
- Message framing: line-delimited, null-delimited, fixed-length, or length-prefixed.
- Maximum command length.
- Delimiter and field rules.
- Case sensitivity.
- Whitespace policy.
- Match type: exact, prefix, field-based, or regular expression.
- Optional checksum or sequence validation.
- Required response or acknowledgment behavior.
- Invalid-character and malformed-message behavior.

### 8.4 XML parsing

XML parser profiles shall define:

- Accepted root element and namespace.
- Required and optional elements.
- Optional schema validation.
- Field or XPath extraction rules.
- Maximum document size.
- Maximum nesting depth.
- Unknown-element behavior.
- Duplicate-command behavior.
- Signature or authentication requirements when supported.

ARC shall use a secure XML parser configuration that disables external entity processing and does not access external resources while parsing.

### 8.5 Custom parser plugins

A custom parser plugin shall use the common parser contract and shall declare:

- Plugin name and version.
- Supported formats.
- Required configuration schema.
- Supported timing features.
- Supported validation features.
- Health and capability status.

---

## 9. Normalized ARC Control Event Model

ARC shall use a transport-neutral event object between parsers, mappings, Device Server translation, and output generation.

### 9.1 Reference event schema

```json
{
  "schemaVersion": "1.0",
  "arcEventId": "generated-unique-id",
  "externalEventId": "provider-event-id-or-null",
  "correlationId": "correlation-id",
  "sourceId": "satellite-east",
  "sourceName": "Satellite East",
  "signalFormat": "scte35",
  "carriageMethod": "mpegts-pid",
  "eventType": "break_start",
  "action": "event",
  "logicalTarget": "network_break",
  "state": null,
  "receivedAt": "UTC timestamp",
  "mediaTimestamp": 0,
  "effectiveAt": "UTC timestamp or null",
  "durationMs": 120000,
  "sequence": 1842,
  "priority": 100,
  "validationStatus": "valid",
  "timingStatus": "scheduled",
  "attributes": {},
  "rawPayloadReference": "audit-payload-id"
}
```

### 9.2 Required fields

Every event shall contain:

- Schema version.
- ARC-generated unique event ID.
- Source identity.
- Signal format.
- Carriage method.
- Normalized event type.
- Arrival time.
- Validation status.
- Timing status.
- Raw-payload reference or an explicit indication that raw storage is disabled.

### 9.3 Event actions

The normalized model shall support:

- Event.
- State ON.
- State OFF.
- Pulse.
- Toggle.
- Cancel scheduled event.
- Clear state.
- Query or status indication.

### 9.4 Event lifecycle

ARC shall track these lifecycle states where applicable:

- Received.
- Parsed.
- Validated.
- Rejected.
- Duplicate.
- Mapped.
- Scheduled.
- Queued.
- Dispatched.
- Accepted by destination.
- Inserted into output stream.
- Confirmed by feedback.
- Timed out.
- Canceled.
- Dropped.

---

## 10. Mapping and Policy Engine

### 10.1 Separation of parsing and mapping

A parser shall describe what an event means. A mapping shall describe what ARC should do with that event.

For example:

```text
Parsed event:
  eventType = break_start
  sourceId = satellite-east

Mapping result:
  Pulse Device Server logical input 5 for 250 ms
  Forward a regenerated SCTE-35 event to Output A
  Suppress the event on Output B
  Publish the event to the live UI
```

### 10.2 Mapping fields

Each mapping shall contain:

- Mapping ID.
- Friendly name.
- Enabled state.
- Source, source group, or global scope.
- Signal format filter.
- Event-type filter.
- Attribute match conditions.
- Optional priority condition.
- Optional time-window condition.
- Destination action list.
- Stop-processing or continue-processing behavior.
- Timing policy.
- Duplicate policy.
- Retrigger policy.
- Error policy.
- Audit level.

### 10.3 Device Server action fields

A Device Server action shall define:

- Device Server connection or plugin instance.
- Logical input bank or target.
- Pin number or logical channel name.
- Action: ON, OFF, pulse, toggle, or clear.
- Pulse duration when applicable.
- Retrigger behavior.
- Dispatch timing policy.
- Confirmation policy.
- Timeout behavior.

### 10.4 Multiple actions

One normalized event may trigger zero, one, or multiple actions. Actions may target Device Server, output streams, APIs, UI notifications, or audit-only destinations.

### 10.5 Mapping conflicts

When multiple mappings target the same state or channel, ARC shall resolve conflicts by configured priority and ordering. The product shall not rely on database retrieval order or thread scheduling for deterministic behavior.

---

## 11. Timing and Audio Alignment

### 11.1 Timing values

ARC shall retain, when available:

- Packet arrival time.
- Parser completion time.
- Input media timestamp.
- Intended effective media timestamp.
- Calculated wall-clock effective time.
- Calculated dispatch time.
- Actual dispatch time.
- Output insertion time.
- Confirmation time.

### 11.2 Timing modes

A source or mapping shall support:

- Immediate on receipt.
- Timestamp aligned.
- Fixed offset.
- Audio-pipeline latency compensated.
- Output-timeline rebased.

### 11.3 Dispatch calculation

Conceptually:

```text
dispatch time =
    effective media time
    + configured event offset
    - estimated downstream command latency
```

The implementation may use a monotonic scheduling clock internally. Wall-clock timestamps shall be retained for observability and cross-service correlation.

### 11.4 Offset behavior

ARC shall support positive and negative offsets within configured safety limits. If an event cannot be dispatched at the calculated time because it arrived late, ARC shall follow the configured late-event policy.

### 11.5 Late-event policies

Supported policies shall include:

- Dispatch immediately.
- Dispatch only if within tolerance.
- Drop and log.
- Audit without execution.
- Route to an exception mapping.

### 11.6 Clock health

ARC shall report clock synchronization status and detected discontinuities. Timed execution shall produce a warning or follow a configured fail policy when the required clock source is unavailable or unhealthy.

### 11.7 Route changes with pending events

When an audio route changes while an event is pending, ARC shall apply one configured policy:

- Preserve the destinations captured when the event was scheduled.
- Apply the current route at execution time.
- Cancel the event and report the cancellation.

The selected policy shall be visible in configuration and audit history.

---

## 12. Duplicate, Debounce, and Retrigger Handling

### 12.1 Deduplication

Deduplication prevents the same transported event from executing more than once. It shall be based on the strongest available identity, such as:

- External event ID.
- Source ID plus event ID.
- Sequence value.
- Payload hash plus time window.
- Configured vendor-specific key.

### 12.2 Debounce

Debounce suppresses rapid state transitions that do not represent distinct intended events. Debounce shall be configured separately from deduplication.

### 12.3 Pulse retrigger policies

A pulse mapping shall define one of these behaviors:

- Ignore while active.
- Restart duration.
- Extend duration.
- Queue another pulse.
- Convert to latched state.
- Reject and report.

### 12.4 Repeated state commands

A repeated ON or OFF may be:

- Ignored as no change.
- Published as a repeated semantic event.
- Re-emitted to selected destinations.

The behavior shall be configurable because some automation workflows depend on repeated trigger occurrences even when the logical state is unchanged.

---

## 13. Device Server Integration

### 13.1 Integration shape

ARC shall use a native Device Server plugin to expose ARC capabilities through the Device Server message contract. The ARC application may run as a separate service, while the Device Server adapter remains a native shared library loaded by Device Server.

The recommended plugin composition for combined routing and GPIO capability is:

- `AbstractControlHardwarePlugin` when using an existing Device Server I/O plugin.
- `AbstractGPIOPlugin` for logical GPI/GPO behavior.
- `AbstractSwitchPlugin` when ARC audio crosspoints are exposed through Device Server.

If ARC owns its communication endpoint directly, the native adapter may manage that connection without an additional Device Server I/O plugin.

### 13.2 ARC-to-plugin protocol

ARC shall expose a versioned local or network protocol for the Device Server plugin. For the POC, this may be a TCP or WebSocket protocol using JSON messages, provided framing and reconnect behavior are deterministic.

The protocol shall support:

- Capability discovery.
- Logical input and output counts.
- Channel names and enabled status.
- Current state snapshot.
- Input state-change events.
- Output command requests.
- Pulse requests.
- Acknowledgments and errors.
- Subscription and resubscription.
- Health status.
- Sequence values.
- Reconnect and reconciliation.

### 13.3 Incoming provider event to Device Server GPI

The normal path shall be:

```text
Incoming signal
 -> extraction
 -> parser validation
 -> normalized ARC event
 -> mapping selection
 -> timing scheduler
 -> logical input action
 -> ARC-to-plugin event
 -> plugin updates pending input mask
 -> plugin publishes Device Server input change
 -> automation system receives GPI change
```

### 13.4 Device Server GPO to outgoing signal

The normal path shall be:

```text
Automation system requests GPO action
 -> Device Server sends output command to ARC plugin
 -> plugin forwards versioned command to ARC
 -> ARC acknowledges receipt
 -> mapping resolves logical output to a signal profile
 -> ARC schedules and generates the signal
 -> signal is injected or sent to selected destinations
 -> ARC reports request, dispatch, insertion, and confirmation status
 -> plugin updates output state according to configured confirmation policy
```

### 13.5 Device Server GPIO messages

The plugin shall implement the applicable Device Server GPIO contract, including:

- `gpio-get-features`.
- `gpio-get-inputs`.
- `gpio-get-outputs`.
- `gpio-set-outputs`.
- `gpio-update-outputs`.
- `gpio-pulse-outputs`.
- `gpio-set-input-debounce` where applicable.
- Input and output change events.

The plugin shall use Device Server mask encoding and decoding utilities rather than define a conflicting mask serialization.

### 13.6 Channel banks and limits

The Device Server GPIO abstraction has a bounded GPIO channel space. ARC shall therefore support multiple logical plugin instances or banks.

The recommended organization is one Device Server plugin instance per automation domain, normally a station, service, or independently controlled destination group. Each instance shall have its own:

- Input and output counts.
- Channel names.
- Mapping scope.
- State snapshot.
- Health status.
- Configuration and audit identity.

The POC shall not assume that every ARC source-event combination receives a permanent unique GPIO pin.

### 13.7 Pin numbering

ARC configuration shall distinguish:

- Internal zero-based indices.
- User-facing labels.
- Device Server one-based feature reporting when configured.

The adapter shall translate consistently and shall test the first and last valid channels.

### 13.8 Logical versus electrical state

ARC and the Device Server boundary shall use logical ON and OFF values. Active-high, active-low, normally-open, normally-closed, relay, open-collector, and device-specific polarity shall be handled by the relevant vendor adapter or output profile.

### 13.9 Pulse handling

ARC shall report or configure:

- Pulse support.
- Native or emulated pulse behavior.
- Minimum, maximum, and default duration.
- Adjustable duration support.
- Retrigger behavior.
- Completion confirmation behavior.

A pulse request shall not be represented as confirmed solely because it was accepted by the plugin.

### 13.10 State reconciliation

After startup or reconnect, the plugin and ARC shall exchange:

- Capability version.
- Channel counts.
- Enabled masks.
- Channel names.
- Current input state.
- Requested output state.
- Observed or confirmed output state.
- Last accepted sequence value where available.

Stale state shall not be published as current without an explicit stale or unknown status.

---

## 14. Signal Generation and Output Injection

### 14.1 Signal Generator

ARC shall include a Signal Generator that converts:

- Device Server GPO requests.
- API requests.
- UI test requests.
- Scheduled events.
- Incoming pass-through events.
- Incoming translated events.

into a supported output signal.

### 14.2 Output formats

The extensible design shall support:

- SCTE-35 in MPEG-TS.
- SCTE-35 representations used by supported HLS output workflows.
- ASCII in a configured private-data channel.
- ASCII through sidecar TCP or UDP.
- XML through a supported sidecar connection.
- Timed metadata.
- Vendor-specific generator plugins.

The POC shall implement only the formats explicitly selected for the POC scope.

### 14.3 Signal profiles

Users shall create reusable signal profiles rather than construct raw messages for every operation. A profile shall contain:

- Profile ID and friendly name.
- Output format.
- Event type.
- Default timing mode.
- Default duration or pulse width.
- Required and optional parameters.
- Destination eligibility.
- Validation rules.
- Format-specific template fields.
- Test mode restrictions.

Example profiles:

- Network Break Start.
- Network Break End.
- Local Avail Start.
- Local Avail End.
- Program Start.
- Program End.
- Station Identification.
- Custom ASCII command.
- Custom XML command.

### 14.4 Route-level signal policy

Each audio route shall define one or more of these cue behaviors:

- Pass unchanged.
- Validate and pass unchanged.
- Parse and regenerate.
- Translate to another format.
- Suppress.
- Consume locally only.
- Fan out to selected destinations.
- Replace with a mapped signal.

### 14.5 Output timeline rebasing

When audio is buffered, transcoded, mixed, decoded, encoded, or repackaged, ARC shall schedule generated or translated signals against the output media timeline. ARC shall not blindly copy an input media timestamp to an unrelated output clock.

### 14.6 Output confirmation levels

ARC shall report the strongest available level:

- Request accepted.
- Queued.
- Generator created payload.
- Mux or sender accepted payload.
- Signal inserted into local output.
- Signal observed by local monitor.
- Downstream acknowledgment received.

Configuration and APIs shall not label a lower confirmation level as physical or downstream execution.

---

## 15. API Requirements

### 15.1 Configuration API

The API shall provide create, read, update, delete, enable, disable, validate, test, export, and import operations as applicable for:

- Sources.
- Destinations.
- Routes.
- Parser profiles.
- Signal profiles.
- Event mappings.
- Device Server connections.
- Device Server logical banks or plugin instances.
- Timing policies.
- Failover policies.

### 15.2 Runtime API

The runtime API shall provide:

- Current source and destination health.
- Current logical GPI and GPO states.
- Event injection through authorized profiles.
- Event cancellation where supported.
- Current scheduled events.
- Recent event history.
- Route changes.
- Test execution with explicit test markers.
- Capability discovery.

### 15.3 Real-time updates

ARC shall expose real-time updates for:

- Source signal receipt.
- Parsed event.
- Mapping decision.
- Device Server dispatch.
- GPI/GPO state changes.
- Output generation and insertion.
- Errors and health changes.

The browser UI may consume these updates through WebSocket or another supported server-push method.

### 15.4 Idempotency and correlation

Mutating API requests that may be retried shall accept an idempotency or correlation key. ARC shall return the ARC event ID and correlation ID in operation responses.

---

## 16. User Interface Requirements

ARC shall include:

### 16.1 GPIO state panel

- Logical inputs and outputs grouped by Device Server instance or bank.
- Friendly channel names.
- ON, OFF, pulsing, stale, and unknown states.
- Visual indication when a pulse occurs.
- Requested versus confirmed output state where supported.

### 16.2 Live event monitor

- Source.
- Format.
- Event type.
- Validation result.
- Effective time.
- Mapping result.
- Destination actions.
- Lifecycle status.
- Correlation ID.

### 16.3 Timing and alignment view

- Arrival time.
- Effective time.
- Scheduled dispatch time.
- Actual dispatch time.
- Measured offset.
- Late or early status.

### 16.4 Mapping editor

- Source and event filters.
- Device Server target.
- Output signal targets.
- Timing, duplicate, and retrigger policies.
- Validation before save.
- Test mode that does not reach production destinations unless explicitly authorized.

### 16.5 Raw diagnostic view

Authorized users shall be able to inspect a sanitized or encoded representation of the original payload and parser diagnostics. Sensitive fields shall be redacted according to policy.

---

## 17. Reliability and Failure Behavior

### 17.1 Source disconnect

Each source shall define whether ARC:

- Retains logical state.
- Clears logical state.
- Marks state unknown or stale.
- Cancels pending scheduled events.
- Preserves pending events.
- Activates a backup source.
- Emits a source-health event.

### 17.2 Device Server disconnect

During Device Server disconnection, ARC shall:

- Continue audio routing unless configured otherwise.
- Retain a bounded audit history.
- Apply a configured queue or drop policy for GPI events.
- Mark Device Server delivery as unavailable.
- Reconcile state after reconnect.
- Avoid replaying expired events unless explicitly configured.

### 17.3 ARC restart

The implementation shall define persistence for:

- Configuration.
- Scheduled future events.
- Current logical states.
- Event deduplication keys.
- Audit history.
- Last sequence values.

The POC may use a local persistence mechanism. Restart behavior shall be deterministic and documented.

### 17.4 Failover source handling

When primary and backup sources carry equivalent cues, ARC shall support event correlation and duplicate suppression across the source pair. Failover shall not automatically trigger the same automation action twice.

### 17.5 Backpressure and overload

ARC shall enforce:

- Maximum payload size.
- Maximum event rate per source.
- Bounded parser and scheduler queues.
- Maximum scheduling horizon.
- Bounded retry behavior.
- Per-destination timeout.
- Circuit breaker or quarantine behavior for repeated malformed input.

Events dropped due to limits shall be counted and audited.

---

## 18. Security Requirements

Production design shall include:

- Authentication for configuration, runtime commands, Device Server communication, and sidecar inputs where supported.
- Authorization separating view, configure, route, test, and execute permissions.
- Source allowlists where applicable.
- Payload validation and size limits.
- Rate limiting.
- Encryption in transit where appropriate.
- Secret storage outside ordinary configuration responses.
- Audit records for manual and API-generated commands.
- Explicit protection against test signals reaching production destinations.
- Secure XML parsing.
- No execution of scripts or shell commands from received signal content.

Security may be simplified for the POC only where the parent epic explicitly allows it. POC shortcuts shall not prevent replacement with production authentication and authorization.

---

## 19. Observability and Audit

### 19.1 Event audit record

ARC shall record, subject to retention policy:

- ARC event ID.
- External event ID.
- Correlation ID.
- Source.
- Raw format and carriage method.
- Raw-payload reference.
- Decode and validation result.
- Arrival and media timing.
- Selected mappings.
- Device Server instance and channel.
- Output destinations.
- Scheduled and actual dispatch times.
- Confirmation level.
- Final status.
- Error or rejection reason.

### 19.2 Logs

Logs shall include component, source or destination ID, plugin instance, event ID, and correlation ID. Logs shall avoid exposing secrets or unrestricted payload content.

### 19.3 Metrics

OpenTelemetry-compatible metrics should include:

- Signals received by source and format.
- Events parsed.
- Validation failures.
- Duplicate events.
- Late events.
- Events dispatched to Device Server.
- Device Server failures and latency.
- Generated events.
- Output insertion success and failure.
- Audio-to-event timing offset.
- Parser and scheduler queue depth.
- Dropped events.
- Reconnect count.
- Clock health.

### 19.4 Tracing

A distributed trace should follow a command from receipt through parser, mapping, scheduler, Device Server dispatch, or output injection. Trace context shall use the event correlation ID where practical.

---

## 20. Configuration Reference Examples

The examples in this section illustrate the intended configuration model. They are not a final API contract.

### 20.1 Incoming SCTE-35 source

```yaml
source:
  id: satellite-east
  friendlyName: Satellite East
  transport:
    type: srt
    mode: listener
    port: 9000
    payload: mpegts
  audio:
    program: auto
    pid: auto
  signaling:
    mode: scte35-mpegts
    pid: auto
    handling:
      - translate-to-device-server
      - forward-by-route-policy
    timingPolicy: source-timestamp-compensated
    lateEventPolicy: dispatch-within-tolerance
    lateToleranceMs: 500
    duplicateWindowMs: 10000
  mappingProfile: satellite-network-cues
```

### 20.2 Mapping event to Device Server GPI

```yaml
mapping:
  id: sat-east-break-start
  enabled: true
  match:
    sourceId: satellite-east
    signalFormat: scte35
    eventType: break_start
  actions:
    - type: device-server-gpi
      instance: station-wxyz
      channelName: Network Break Start
      channelIndex: 4
      action: pulse
      pulseDurationMs: 250
      timingPolicy: event-effective-time
      confirmationPolicy: plugin-accepted
  duplicatePolicy: ignore
  retriggerPolicy: restart
```

### 20.3 Device Server GPO to output SCTE-35

```yaml
mapping:
  id: station-wxyz-local-break
  enabled: true
  match:
    type: device-server-gpo
    instance: station-wxyz
    channelName: Local Break Start
  actions:
    - type: signal-profile
      profileId: scte35-local-avail-start
      destinations:
        - wxyz-network-return
      timingPolicy: output-timeline
```

---

## 21. POC Implementation Scope

### 21.1 Required POC capabilities

Unless superseded by the parent epic, the initial POC shall demonstrate:

1. At least one live or recorded MPEG-TS-based source received through a selected network transport.
2. Audio extraction and routing to at least one destination.
3. SCTE-35 extraction from a configured or discovered PID.
4. Conversion of selected SCTE-35 events into normalized ARC events.
5. Mapping of normalized events to logical Device Server GPI actions.
6. A native Device Server plugin that exposes configured GPI and GPO channels.
7. Receipt of Device Server GPO commands by ARC.
8. Mapping of at least one GPO command to a generated output signal.
9. SCTE-35 generation and insertion into an MPEG-TS output, if selected as the POC output format.
10. A browser interface showing live events and logical GPIO status.
11. REST configuration and runtime APIs.
12. Real-time browser updates.
13. Audit logs and basic OpenTelemetry-compatible metrics.
14. Deterministic reconnect and state reconciliation between ARC and the Device Server plugin.
15. Docker deployment on Ubuntu Server 24.04 for the ARC service.
16. Build instructions for the Device Server plugin on each selected Device Server platform.

### 21.2 Recommended POC simplifications

The POC may:

- Use JSON over TCP or WebSocket between ARC and its Device Server plugin.
- Use a local data store.
- Support a limited set of normalized event types.
- Start with one Device Server plugin instance.
- Implement SCTE-35 before optional ASCII and XML formats.
- Use simple credentials allowed by the parent epic.

The implementation shall not hard-code these simplifications into interfaces in a way that prevents later expansion.

### 21.3 Suggested initial normalized event types

- `break_start`.
- `break_end`.
- `program_start`.
- `program_end`.
- `station_identification`.
- `custom_trigger`.
- `source_online`.
- `source_offline`.

---

## 22. Functional Acceptance Criteria

### 22.1 Source and extraction

- ARC can configure a source with separate transport, container, audio, and signaling settings.
- ARC can discover or accept an explicitly configured signaling PID for an MPEG-TS source.
- ARC preserves raw signal data and relevant timing context.
- A parser error does not crash the audio pipeline.
- Unsupported signaling is reported clearly.

### 22.2 Parsing and normalization

- A valid supported SCTE-35 cue produces a normalized ARC event.
- A malformed cue produces a structured rejection and audit record.
- Event identity, timing, duration, cancel status, and supported descriptors are preserved.
- ASCII and XML parsers, when included in the POC, use configured framing and validation profiles.
- Parsers do not contain hard-coded Device Server pin assignments.

### 22.3 Mapping

- An administrator can map an event type to a Device Server instance and channel.
- One event can trigger multiple configured actions.
- Mapping priority and ordering are deterministic.
- Disabled mappings do not execute.
- Mapping test mode cannot reach production destinations unless explicitly authorized.

### 22.4 Timing

- Immediate and timestamp-aligned dispatch modes operate as configured.
- Positive and negative offsets are applied within configured limits.
- Late events follow the selected late-event policy.
- Actual dispatch time and calculated timing offset are recorded.
- Output cues are bound to the output media timeline after repackaging or transcoding.

### 22.5 Deduplication and pulses

- Repeated copies of the same event are handled by the configured duplicate policy.
- Debounce and deduplication are configured and tested independently.
- Pulse duration and retrigger behavior are deterministic.
- Repeated state commands follow the configured behavior.

### 22.6 Device Server

- The native plugin loads with the selected Device Server build.
- The plugin reports correct input and output counts, names, numbering, and pulse capability.
- The first and last valid input and output channels operate correctly.
- A normalized ARC event produces the correct Device Server input-mask change.
- Device Server output set, update, and pulse requests reach ARC.
- Requested output state is not reported as confirmed beyond the configured confirmation level.
- ARC and the plugin reconcile state after reconnect.
- Invalid or out-of-range masks are rejected safely.

### 22.7 Output generation

- A configured Device Server GPO action selects the expected signal profile.
- Required signal fields are validated before generation.
- The signal is sent or inserted only into configured destinations.
- Route policy can pass, regenerate, translate, suppress, or consume a supported cue.
- Output status distinguishes accepted, queued, inserted, observed, and acknowledged states where those levels are available.

### 22.8 Monitoring

- The UI displays logical GPI and GPO states.
- The UI displays received events and mapping results in real time.
- Audit history correlates input, mapping, Device Server action, and output action.
- Metrics identify parsing failures, late events, duplicates, dispatch failures, and dropped events.

### 22.9 Reliability

- Source, ARC-to-plugin, and Device Server reconnect scenarios are tested.
- Queues are bounded.
- Expired events are not replayed after reconnect unless configured.
- Malformed input and excessive event rates do not cause unbounded resource use.
- Failover duplicate suppression prevents double execution for correlated primary and backup cues.

---

## 23. Required Test Scenarios

The automated test suite shall include:

1. Valid immediate cue.
2. Valid future-timed cue.
3. Cue with duration.
4. Cancel of a pending event.
5. Duplicate event ID.
6. Duplicate payload without an event ID.
7. Invalid checksum or CRC.
8. Unknown command or descriptor.
9. Event arriving within late tolerance.
10. Event arriving outside late tolerance.
11. Positive timing offset.
12. Negative timing offset.
13. Pulse retrigger for every supported policy.
14. Repeated ON and repeated OFF.
15. First and last Device Server channels.
16. Out-of-range Device Server mask.
17. Device Server disconnect during dispatch.
18. ARC-to-plugin reconnect and state snapshot.
19. ARC restart with pending future event.
20. Route change while an event is pending.
21. Primary and backup sources sending the same event.
22. Output timeline discontinuity.
23. Malformed ASCII framing.
24. Oversized XML input.
25. XML external entity attempt.
26. Event-rate limit exceeded.
27. Output generator validation failure.
28. Signal inserted into the correct output only.
29. Manual test event with production-output protection.
30. Audit correlation from raw input to final action.

---

## 24. Open Product and Architecture Decisions

The following decisions shall be resolved before production acceptance. POC defaults may be selected and documented.

1. Which actual provider formats and carriage methods are required for the first release?
2. Which input and output transports must support SCTE-35 in the first release?
3. Will the first POC use recorded provider captures, generated test streams, or both?
4. What Device Server platform and build toolchain will be used for plugin validation?
5. How many logical GPI and GPO channels are required per automation domain?
6. Is one plugin instance per station the required default?
7. Which normalized event types are mandatory beyond break start and break end?
8. What timing accuracy is required for Device Server dispatch and output insertion?
9. What clock synchronization method is required in production?
10. What event retention and raw-payload retention periods are required?
11. Which confirmation level is available for each output type?
12. Should scheduled events survive ARC restart?
13. What behavior is required when a source disconnects while a logical input is latched?
14. Which ASCII and XML vendor formats are required?
15. Will Device Server expose ARC crosspoints through `AbstractSwitchPlugin` in the POC or a later phase?
16. What authentication method will protect the production ARC-to-plugin protocol?
17. What licensing limits apply to sources, destinations, logical GPIO banks, and generated signals?

---

## 25. Information Required from Signal Providers

For each satellite, network, codec, or contribution provider, obtain:

- Protocol or carriage specification.
- Representative packet capture or recorded stream.
- Container and codec description.
- Program and PID information where applicable.
- Example break-start and break-end commands.
- Timing and pre-roll behavior.
- Event identity and sequence behavior.
- Duplicate or repetition behavior.
- Cancel behavior.
- Duration behavior.
- Required acknowledgment behavior.
- Sidecar connection details where applicable.
- Failover behavior.
- Test schedule or test-stream availability.

A provider shall not be marked supported based only on the phrase "embedded GPIO." Support requires a defined and tested carriage and command format.

---

## 26. Definition of Done

Signal handling and Device Server integration are complete when:

- Required source formats are documented and tested.
- ARC separates transport, container, signaling, parsing, mapping, and execution concerns.
- The normalized event schema is versioned and documented.
- Source and destination signal policies are configurable.
- Device Server GPI and GPO paths are functional and tested.
- Timing, duplicate, debounce, retrigger, reconnect, and failover policies are deterministic.
- Requested and confirmed output states are represented accurately.
- Output generation and injection operate against the correct output timeline.
- Real-time monitoring, audit history, logs, metrics, and traces are available.
- Security requirements for the selected release are implemented.
- Automated tests cover normal, malformed, late, duplicate, reconnect, and overload cases.
- Deployment and plugin build instructions are reproducible.
- All POC shortcuts and remaining production gaps are documented.

---

## 27. Implementation Guidance for AI-Assisted POC Development

When this document is supplied to an AI coding assistant, the assistant shall:

1. Treat this specification as the required behavior, not as sample prose.
2. Produce a phased implementation plan before generating the complete application.
3. Identify all assumptions and unresolved items from Section 24.
4. Avoid inventing undocumented provider protocols.
5. Use interfaces and plugins for format-specific parsers and generators.
6. Keep the normalized event schema independent of transport libraries and Device Server pin numbers.
7. Keep audio routing operational when a signal parser fails, unless configured otherwise.
8. Use bounded queues and explicit cancellation for scheduled tasks.
9. Use monotonic time for internal scheduling and UTC timestamps for audit records.
10. Preserve raw test fixtures for parser regression tests.
11. Mock the Device Server boundary for automated tests while maintaining a separate native plugin project.
12. Generate API documentation and configuration examples.
13. Include unit, integration, reconnect, timing, and malformed-input tests.
14. Mark any unimplemented capability clearly in both the API and UI.
15. Do not represent a transmitted command as confirmed unless feedback supports that status.
16. Provide reproducible Docker and local-development instructions.
17. Maintain the Device Server plugin in a separate deployable folder as required by the parent epic.

---

## 28. Related Project Documents

This specification is intended to be used with:

- `DRAFT - Audio Routing Controller (ARC).md`
- `VENDOR_GPIO_BEHAVIOR.md`
- `PLUGIN_FOR_NEW_ROUTER.md`

Where a conflict exists, the Jira epic and approved architecture decisions take precedence. Unresolved conflicts shall be documented rather than silently implemented.
