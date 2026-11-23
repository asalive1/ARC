Supplemental Doc 1

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


Supplemental Doc 2:

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


Supplemental Doc 3:

# ARC Audio Output Destination Handling Specification

**Document status:** Draft for product and technical review  
**Application:** Audio Routing Controller (ARC)  
**Document role:** Jira attachment, engineering requirements, POC scope, and AI-assisted implementation specification  
**Related documents:** ARC Epic, ARC Audio Input Source Handling Specification, and ARC Signal Handling and WideOrbit Device Server Integration Specification

---

## 1. Purpose

This document defines how ARC shall configure, validate, activate, route, encode, packetize, publish, monitor, recover, and expose audio output destinations. It also defines how destination audio, metadata, embedded signaling, and Device Server-generated control events remain aligned.

ARC shall model an output destination as a layered resource:

```text
Destination identity and lifecycle
    -> routed source selection
        -> audio processing and delay
            -> codec and output format
                -> payload/container
                    -> transport or local device
                        -> signaling and metadata insertion
                            -> transmission, monitoring, and recovery
```

A destination is not merely an IP address and port. It owns an explicit output contract, connection behavior, no-source behavior, health model, cue policy, lifecycle, and recovery policy.

---

## 2. Product Decisions Summary

### 2.1 Required POC destination types

The initial POC shall support:

- SRT carrying MPEG-TS.
- UDP carrying MPEG-TS.
- RTP audio.
- ALSA hardware output.
- Windows WASAPI shared-mode output.
- Null/test destination.

File recording is not part of the POC despite having appeared in the initial recommended list.

HLS publishing and Icecast publishing are included in the destination design and requirements. HLS shall be generated and hosted directly by ARC. Icecast shall publish to an existing Icecast server. Because these outputs require service infrastructure, their POC completion depends on the stated test assets and deployment environment.

AES67 output is post-POC unless suitable clocking and representative receiving infrastructure become available.

### 2.2 Global destination namespace

ARC shall maintain one application-wide destination catalog.

- Any compatible source may route to any destination.
- Destination classifications and tags are organizational only.
- Destinations shall not be partitioned by station, tenant, facility, or automation domain in the POC.

### 2.3 Routing cardinality

- A destination may have zero or one active source.
- A source may feed any number of destinations, subject to host resources and license limits.
- Mixing multiple sources at a destination is not part of the POC.
- Routing a second source to an occupied destination requires an explicit replace operation.

### 2.4 Route transition

The POC transition is an immediate hard cut. ARC shall preserve the destination encoder and transport session when technically possible so downstream systems do not reconnect unnecessarily.

### 2.5 Default output behavior

- Default audio format: 48 kHz stereo.
- Codec and bitrate: selected by destination.
- Default no-source behavior for persistent destinations: continuous digital silence.
- Source failure: preserve the route and send silence by default.
- Source recovery: resume program audio automatically.
- Destination failure: preserve the route, isolate the failed output, and retry indefinitely.

---

## 3. Terminology

### 3.1 Destination

A configured ARC output resource with a stable identifier, output media contract, transport or device configuration, signaling policy, health, lifecycle, and current route.

### 3.2 Output contract

The codec, sample rate, sample format, channel layout, bitrate, packet duration, payload/container, and transport expected at the destination boundary.

### 3.3 Persistent destination

A destination that remains running before and between routes, normally transmitting silence while no program source is selected.

### 3.4 Destination-local restart

Rebuilding only the affected destination pipeline. It shall not restart ARC, sources, other destinations, APIs, UI services, or Device Server integration.

### 3.5 Explicit replace

An atomic routing operation that replaces the existing source assigned to a destination with another source.

### 3.6 Confirmed destination

A destination for which the strongest required downstream status for that protocol or configured policy has been observed.

---

## 4. Destination Resource Model

Every destination shall contain these logical sections. Adapters shall expose only applicable fields.

```yaml
destination:
  identity: {}
  classification: {}
  lifecycle: {}
  routeBehavior: {}
  processing: {}
  delay: {}
  outputFormat: {}
  encoder: {}
  payload: {}
  connection: {}
  transport: {}
  signaling: {}
  metadata: {}
  authentication: {}
  monitoring: {}
  recovery: {}
  licensing: {}
```

### 4.1 Identity

Required fields:

- Immutable ARC destination ID.
- Unique friendly name.
- Optional description.
- Optional tags.
- Schema version.
- Configuration revision.
- Creation and modification timestamps.

Routes, metrics, events, and APIs shall reference the immutable destination ID.

### 4.2 Classification

Each destination shall support:

- Environment: production, test, or development.
- Role: primary, backup, monitor, recorder, or temporary.

Classifications shall support organization and permissions. They shall not restrict routing.

### 4.3 Administrative state

Each destination shall include:

- Enabled or disabled.
- Autostart enabled or disabled.
- Required or optional at ARC startup.
- Draft revision.
- Active revision.
- Current running state.

The POC default shall be autostart enabled. This setting shall remain configurable per destination.

---

## 5. Destination Lifecycle

ARC shall distinguish:

1. **Saved:** A draft configuration exists.
2. **Validated:** Schema, compatibility, and applicable endpoint tests passed.
3. **Active:** A validated revision was applied.
4. **Running:** The destination pipeline was started.

### 5.1 Runtime states

A destination shall report:

- Disabled.
- Stopped.
- Starting.
- Waiting for route.
- Waiting for peer.
- Connecting.
- Running silence.
- Transmitting.
- Degraded.
- Reconnecting.
- Failed.
- Stopping.
- Unknown.

Waiting for Peer shall not be a failed state unless configuration requires a peer to be present within a defined interval.

### 5.2 Editing a running destination

Changes shall be written to a draft and applied explicitly. ARC shall classify changes as:

- Live-applicable.
- Requires destination-local restart.
- Invalid while active.

A disruptive change shall restart only that destination pipeline.

### 5.3 Rollback

The POC shall support rollback to the immediately previous active destination revision. Rollback shall validate the revision, identify restart requirements, and preserve an audit record.

---

## 6. Destination Operations

The POC UI and API shall expose:

- Create.
- Clone.
- Read.
- Update draft.
- Validate.
- Test endpoint reachability.
- Test authentication.
- Test bind/listen availability.
- Apply.
- Roll back.
- Enable.
- Disable.
- Start.
- Stop.
- Destination-local restart.
- Route source.
- Explicitly replace routed source.
- Remove route.
- Send authorized test tone.
- Send silence.
- Start temporary output test.
- Monitor post-processing audio.
- Delete.
- View health and statistics.

Long-running operations shall be asynchronous and return operation identifiers.

---

## 7. Routing Behavior

### 7.1 One source per destination

A destination shall accept zero or one routed source. ARC shall reject an ordinary route-add request when the destination is occupied.

The caller shall use an explicit replace operation containing:

- Destination ID.
- Expected current source ID or route revision.
- Replacement source ID.
- Requested transition mode.
- Correlation or idempotency key.

### 7.2 Atomic replacement

Replacement shall appear atomically in the route model. ARC shall avoid a partially configured state where routing indicates one source while another is still active without an explicit transition status.

### 7.3 Hard-cut transition

The POC shall use an immediate hard cut. ARC shall:

- Prepare the replacement media path when possible.
- Preserve the destination encoder and transport session where compatible.
- Change audio at a buffer boundary determined by the destination pipeline.
- Record actual switch time.
- Rebase cue and metadata timing to the destination timeline.

Crossfade and silence-gap transitions are post-POC extensions.

### 7.4 No-source policy

Each destination shall support:

- Continuous digital silence.
- Stop transmitting.

Persistent destinations default to digital silence. Signaling while silent shall be configurable:

- Continue eligible signaling.
- Suppress signaling.
- Permit only generated/manual control signals.

ARC shall never hold and repeat the last audio frame indefinitely as no-source behavior.

### 7.5 Source failure policy

Each destination shall support a configurable policy. POC default:

- Preserve the route.
- Maintain the destination transport when possible.
- Replace program audio with digital silence.
- Mark the destination as carrying source-failure silence.
- Automatically restore program audio after source recovery.

Automatic fallback-source routing is post-POC.

---

## 8. Output Audio Contract

Every destination shall define independently:

- Codec.
- Sample rate.
- Bit depth or sample format.
- Channel layout.
- Bitrate where applicable.
- Packet duration where applicable.
- Payload or container.
- Transport or local-device backend.

Defaults:

- 48 kHz.
- Stereo.
- Destination-selected codec and bitrate.
- Maximum two channels for the POC.

### 8.1 Codec capability reporting

ARC shall report codecs dynamically according to installed components, operating system, architecture, and adapter capability.

The UI and API shall not offer unavailable codecs as valid selections.

### 8.2 Required POC output codecs

Mandatory:

- PCM L16.
- PCM L24.

Where required by destination adapters and available in the installed build, the design shall support:

- AAC-LC.
- HE-AAC.
- MP3.
- Opus.

RTP, ALSA, SRT, and local audio output are transports or output mechanisms, not codecs. Each shall use an explicitly selected compatible codec or sample format.

### 8.3 Unsupported codecs

Unsupported codec selection shall fail validation. ARC shall identify supported alternatives and preserve the draft configuration.

---

## 9. Pass-Through and Encoding Policy

Each destination shall select one policy:

- Prefer pass-through.
- Force encode.
- Pass-through only.

### 9.1 Prefer pass-through

ARC shall pass compatible encoded media without unnecessary decode/re-encode. When the source is incompatible, ARC shall decode, normalize, and encode automatically if a supported path exists.

### 9.2 Force encode

ARC shall use the destination encoder even when pass-through is possible. This supports consistent output parameters.

### 9.3 Pass-through only

An incompatible source shall be rejected. ARC shall not silently override the selected policy.

### 9.4 Route planning information

Before route activation, ARC shall report:

- Pass-through or transcode path.
- Effective codec and format.
- Expected additional latency.
- Estimated processing cost.
- Channel conversion.
- Known quality loss.

---

## 10. Channel Handling

The POC primary destination layout is stereo. The extensible model shall also represent mono and dual mono.

- Maximum channels: two.
- Stereo pair locked together.
- Mono source duplicated for stereo output.
- Stereo source downmixed for mono output.
- Dual mono remains one destination object.
- Dual-mono channels may have independent labels and gains.
- Independent routing of unrelated sources into left and right channels is mixing and is not supported in the POC.

---

## 11. Destination Audio Processing

The POC shall provide:

- Output gain.
- Mute.
- Channel mapping.
- Channel swap.
- Mono duplication.
- Stereo downmix.
- Sample-rate conversion.
- Configurable output delay.
- Authorized test tone.

Output gain shall be independent of source gain.

Deferred processing includes:

- Automatic gain control.
- Loudness normalization.
- Limiter.
- General filters.
- General polarity inversion.

### 11.1 Mute distinctions

ARC shall distinguish:

- Administrative destination mute.
- Source-failure silence.
- Destination health-policy mute.
- Monitor-session mute.

Monitor mute shall not change program output.

---

## 12. Destination Audio Delay

The POC shall provide a delay configured per destination.

- Delay shall be independent of cue offset.
- Delay shall use bounded memory.
- Changes shall report whether they apply live or require pipeline restart.
- Delay shall be included in route planning, latency reporting, and cue scheduling.

The maximum delay shall be deployment/resource configurable. The implementation shall publish the POC-tested maximum for each supported deployment profile.

A route-specific audio delay is post-POC functionality.

---

## 13. Network Binding

Network destinations shall distinguish:

- Local bind address.
- Local source port.
- Remote host or address.
- Remote port.
- Stable network interface ID.
- Selected interface address.
- IPv4 or IPv6.
- Unicast or multicast.
- Multicast group.
- Multicast TTL.
- DSCP or traffic class.
- Packet size.
- Source-specific multicast applicability.

One multicast destination object shall send to one multicast group. Sending identical audio to another group requires another destination.

---

## 14. SRT Destination Requirements

### 14.1 Required modes

- Caller.
- Listener.

Rendezvous is post-POC.

### 14.2 Required fields

- Mode.
- Local bind address and port.
- Remote host and port where applicable.
- Network interface.
- Address family.
- Stream ID.
- Optional passphrase secret reference.
- SRT latency.
- Packet size.
- Timestamp delivery setting.
- Bandwidth/overhead policy.
- Reconnect policy.
- Cue injection policy.
- Payload/container.

MPEG-TS is the only mandatory POC SRT payload.

### 14.3 Listener behavior

- One configured downstream peer per destination.
- Validate Stream ID when configured.
- Reject unknown or mismatched Stream ID.
- Do not create destinations dynamically.
- Maintain the destination pipeline while the peer reconnects.
- Waiting for Peer is non-failed by default.

### 14.4 Persistence without a source

When configured for continuous silence, an SRT destination shall establish or await its transport session before a source route is assigned.

---

## 15. UDP MPEG-TS Destination Requirements

The POC mandatory mode is unicast. The design shall also support multicast.

Required fields:

- Remote destination or multicast group.
- Remote port.
- Local bind address and interface.
- Address family.
- TTL for multicast.
- DSCP or traffic class.
- Packet size.
- MPEG-TS mux rate.
- PAT/PMT interval.
- PCR interval.
- Constant or variable bitrate mode.
- Null-packet padding enabled/disabled.
- Signaling PID configuration.
- Continuity monitoring.

Advanced mux fields shall be grouped separately in the UI.

Constant-bitrate MPEG-TS with null-packet padding shall be configurable.

---

## 16. RTP Destination Requirements

### 16.1 POC functions

- Manual configuration.
- Generated downloadable SDP.

Published SDP URL, SAP announcement, RTCP, and SRTP are post-POC unless required by an integration.

### 16.2 Required fields

- Destination address.
- Destination port.
- Local bind/interface.
- Unicast or multicast.
- Payload type.
- Codec.
- Clock rate.
- Channel count.
- Packet duration.
- SSRC.
- Timestamp behavior.
- TTL and DSCP where applicable.

Although only selected fields were explicitly marked by Product Management, clock rate, channel count, packet duration, SSRC, timestamp behavior, TTL, and DSCP shall remain required or conditionally required because they define a deterministic RTP output and generated SDP.

### 16.3 SDP generation

Generated SDP shall match the active configuration and include sufficient session/media information for the supported receiving test endpoint.

---

## 17. HLS Publishing Requirements

ARC shall generate HLS manifests and media segments and host them directly.

### 17.1 Required fields

- Target directory.
- Public base URL.
- Segment duration.
- Playlist window.
- Cleanup behavior.
- Segment type: MPEG-TS or fMP4.
- Mode: standard HLS or Low-Latency HLS, configurable when capability is installed.
- Maximum retained storage.
- Authentication/access policy.
- Codec and bitrate.
- Metadata and cue policy.

### 17.2 Hosting responsibility

ARC shall provide or integrate an internal hosting surface for generated files. The implementation shall separate generation from HTTP serving so a future deployment may use an external web server or origin without replacing the media pipeline.

### 17.3 Storage management

- Storage limits shall be configurable.
- Cleanup shall never remove files still referenced by the active playlist.
- Failure to write or serve media shall affect HLS destination health.
- Retention and cleanup status shall be observable.

### 17.4 Authentication

Authentication responsibility was not fully specified. POC assumption: ARC's existing UI/API authentication does not automatically secure public HLS media URLs. HLS access control shall be an explicit destination setting and capability. This assumption requires security review before production.

---

## 18. Icecast Publishing Requirements

ARC shall publish as a source client to an existing Icecast server. ARC shall not operate the Icecast server.

### 18.1 Required codecs

- AAC.
- MP3.
- Opus.

### 18.2 Required fields

- Complete server and mount-point URL.
- Source username when required.
- Source-password secret reference.
- TLS validation policy.
- Codec and bitrate.
- Stream name.
- Description.
- Optional genre.
- Public/private flag.
- ICY metadata enabled.
- Now Playing metadata enabled.
- Optional custom headers.
- Reconnect policy.

### 18.3 Metadata

ARC shall send configured stream information, ICY metadata, and Now Playing metadata. Metadata may originate from source metadata, Device Server, automation API, or manual/mapped events.

---

## 19. Linux Local Audio Output

The POC shall support:

- ALSA hardware output.
- JACK output where available in the deployment.

ALSA hardware is mandatory for end-to-end demonstration. ALSA plug, PulseAudio, and PipeWire are post-POC extensions unless delivered by the selected media framework without separate product risk.

Required fields:

- Backend.
- Stable device ID.
- Display name.
- Selected stereo output pair.
- Requested sample rate and format.
- Buffer/period settings where applicable.
- Format-negotiation policy.

A returning device shall reconnect by stable ID.

---

## 20. Windows Local Audio Output

The POC shall support:

- WASAPI shared mode.
- Explicit endpoint selection.

Required fields:

- Stable endpoint ID.
- Display name.
- Shared mode.
- Selected stereo output pair.
- Requested or negotiated format.
- Buffer duration.
- Format-negotiation policy.

WASAPI exclusive mode and default-device following are post-POC.

### 20.1 Device disappearance

If a local output disappears:

- Preserve the route.
- Enter Failed.
- Discard output through a bounded nonblocking path.
- Avoid blocking the source or other destinations.
- Reconnect and resume automatically by stable ID.

---

## 21. Local-Device Format Negotiation

When a device cannot accept the configured output format, ARC shall convert automatically when supported and display:

- Requested format.
- Effective device format.
- Conversion path.
- Estimated latency and processing cost.

If conversion is unavailable, activation shall fail with an actionable error.

---

## 22. Destination Clocking and Synchronization

Clock selection shall be automatic by destination type:

- Local device clock for local audio where applicable.
- Protocol/media clock for RTP and timed transports.
- System monotonic clock for scheduling and fallback.
- Container timeline for muxed output.

ARC shall report active clock source and clock health.

### 22.1 Drift correction

ARC shall perform bounded resampling or equivalent drift correction when source and destination clocks differ and decoded audio is available.

### 22.2 Clock loss

When the preferred clock is lost, ARC shall fall back to system monotonic clock when the adapter supports safe continuation. The destination shall be marked Degraded and the fallback shall be logged.

If safe fallback is impossible, the destination shall fail rather than emit invalid media.

---

## 23. Startup Behavior

Destination startup shall be configurable. Default: enabled destinations start with ARC.

Persistent destinations configured for continuous silence shall establish their connection, host surface, or listener before a route is assigned.

Startup failure of an optional destination shall not prevent unrelated ARC services from starting. Required destinations may contribute to overall application readiness without terminating the process.

---

## 24. Validation and Testing

ARC shall provide all applicable operations:

- Schema and compatibility validation.
- Endpoint reachability test.
- Authentication test.
- Bind/listen availability test.
- Test tone.
- Silence generation.
- Temporary output generation.
- Local loopback verification when supported.
- Downstream acknowledgment inspection.
- Effective codec/container display.
- Estimated bitrate and latency.

### 24.1 Test-tone protection

Sending test tone to a routed production destination requires explicit authorization and confirmation. The operation shall identify the impacted destination and current routed source and shall be audited.

---

## 25. Downstream Confirmation

ARC shall expose only status levels supported by the selected destination:

- Configuration valid.
- Pipeline running.
- Connected.
- Packets sent.
- Bytes sent.
- Downstream peer connected.
- Transport acknowledgment available.
- Remote application acknowledgment.
- Playback confirmed.

Successful transmission shall not be labeled playback confirmation.

### 25.1 Required confirmation policy

Product decision: failure to obtain required downstream acknowledgment shall fail the destination.

Because not every protocol provides acknowledgments, each destination shall define its required confirmation level. For UDP and other unacknowledged protocols, the highest technically available level may be packets/bytes sent. ARC shall not require an impossible acknowledgment unless an external monitor is configured.

A destination shall fail only when its configured required level is not reached within the configured interval.

---

## 26. Destination Health Model

ARC shall track independently:

- Pipeline state.
- Encoder state.
- Mux state.
- Network or device connection.
- Packet transmission.
- Downstream connection.
- Required confirmation state.
- Queue and buffer state.
- Clock and timing.
- Signaling insertion.
- Metadata transmission.
- Overall health.

Each dimension shall report:

- Healthy, degraded, failed, or unknown.
- Green, yellow, or red.
- Reason code.
- Human-readable diagnostic.
- First-observed and last-updated timestamps.

---

## 27. Failure and Recovery

When a destination fails, ARC shall:

- Preserve the route.
- Retry indefinitely using configured backoff.
- Alert on the first failure transition.
- Isolate the destination.
- Avoid blocking or restarting its source.
- Avoid affecting other destinations.
- Revalidate the endpoint after reconnect.
- Resume output using the preserved route.

### 27.1 Retry defaults

- Initial retry: 5 seconds.
- Exponential backoff with jitter.
- Maximum retry: 60 seconds, configurable.
- Unlimited attempts.
- Alert on first failure.
- Reset backoff after stable operation.

### 27.2 Listener behavior

Waiting for Peer is a valid operational state and shall not fail by default. A destination may define a peer-required timeout that converts it to Failed.

---

## 28. Output Queues and Backpressure

Every destination shall have independently bounded queues.

- One slow destination shall never block the source or other destinations.
- Adapter-specific policy shall control drop/rebuild behavior.
- Latency may increase only to the configured maximum.
- Beyond the bound, ARC shall recover or drop according to the adapter policy.
- Unbounded buffering is prohibited.

ARC shall expose queue depth, queue duration, overflow, underflow, and drops where supported.

---

## 29. Cue and Metadata Handling

Each destination shall support applicable signal policies:

- Pass original cue.
- Validate and pass.
- Regenerate.
- Translate format.
- Suppress.
- Consume locally only.
- Inject a generated GPO-derived cue.
- Forward Now Playing metadata.
- Suppress metadata.

Policy shall be configured at the destination, with a route-level override.

Only capabilities supported by the destination adapter shall appear in configuration.

---

## 30. Audio and Cue Alignment

ARC shall support:

- Destination audio delay.
- Separate destination cue offset.
- Output media timeline rebasing.
- Configured late-event policy.

The UI and API shall show:

- Audio pipeline latency.
- Configured audio delay.
- Cue offset.
- Estimated cue execution position.
- Measured insertion time where available.
- Late-cue status.

If a cue cannot be inserted at the requested time, ARC shall follow the configured late-event policy from the signal-handling specification.

---

## 31. Metadata Output

ARC's extensible metadata mapping shall support applicable destination formats:

- ICY Now Playing.
- HLS timed metadata.
- ID3.
- SCTE-35.
- Custom ASCII.
- XML.
- Device Server metadata.

Metadata may originate from:

- Incoming source metadata.
- Device Server.
- Automation API.
- Manual UI entry.
- Mapping rules.

Unsupported formats shall not be selectable for a destination.

---

## 32. Recording Destination

File recording is not part of the POC.

The resource model shall not prevent a later recording adapter supporting segmented and retained audio. Recording formats, segmentation, retention, disk quotas, file naming, atomic completion, and metadata require a separate product review before implementation.

---

## 33. Null/Test Destination

The POC shall include a null/test destination that accepts and measures media without external transmission.

It shall validate:

- Route establishment.
- Throughput.
- Decode and normalization path.
- Output-format conversion.
- Timing.
- Cue mapping.
- Metadata mapping.
- Resource consumption.

A destination explicitly designated as internal test tooling shall not count toward destination licensing.

---

## 34. Destination Monitoring

The UI and API shall expose:

- Routed source.
- Route revision.
- Encoder format.
- Effective bitrate.
- Packets and bytes sent.
- Queue level and duration.
- Buffer underflow/overflow.
- Connection state.
- Downstream peer.
- Required confirmation state.
- Output level meters.
- Estimated latency.
- Cue insertion events.
- Metadata events.
- Destination errors.

Operators shall be able to monitor post-processing destination audio using authenticated preview infrastructure.

The POC allows one browser monitor at a time across source and destination monitoring.

---

## 35. Authentication and Secrets

The destination authentication model shall support as required by adapters:

- Username/password.
- HTTP Basic.
- Bearer token.
- API key.
- Custom headers.
- SRT passphrase.
- Icecast source password.
- Client certificate.
- Custom certificate authority.

Secrets shall use the same policy as sources:

- Store separately.
- Reference from configuration.
- Never return through the API.
- Redact from logs and diagnostics.
- Report only whether configured and a non-sensitive reference ID.

---

## 36. Persistence

Persist across ARC restart:

- Destination definitions.
- Draft and active revisions.
- Enabled state.
- Current routes.
- Encoder settings.
- Cue policies.
- Metadata policies.
- Authentication references.
- Startup and no-source behavior.

Rebuild or rediscover:

- Runtime operating state.
- Runtime statistics.
- Downstream peers.
- Clock state.
- Queue state.
- Browser monitor session.

---

## 37. Licensing

Licensing shall count enabled destinations. Local and network destinations shall count equally unless a future commercial model defines different entitlements.

At the license limit:

- ARC may create and save additional destination configurations.
- ARC shall prevent enabling a destination above the licensed limit.
- Existing enabled destinations remain operational.
- The API and UI shall report the limit and current count.

Null/test destinations designated as internal test tooling shall not count.

---

## 38. Scale and Resource Targets

Initial targets:

- Maximum configured destinations: 64.
- Maximum active destinations: 64, subject to capacity qualification.
- Expected average: approximately 16.
- Maximum destinations per source: no product-defined fixed limit.
- Maximum source per destination: one.
- Simultaneous destination tests: four.
- POC browser monitors: one across source and destination monitoring.

No fixed aggregate output bandwidth or encoder count has been selected. ARC shall measure:

- Aggregate output bandwidth.
- Encoder count and CPU cost.
- Memory and queue usage.
- Per-destination media cost.
- Cue/metadata processing cost.
- Preview cost.

Capacity testing shall establish supported host profiles.

---

## 39. API Requirements

The API shall provide resources or equivalent operations for:

- Destinations.
- Destination revisions.
- Validation and endpoint tests.
- Lifecycle operations.
- Routes and explicit replacement.
- Destination capability discovery.
- Health and statistics.
- Monitor sessions.
- Test-tone/silence operations.
- Cue and metadata policy.
- Secret references.

### 39.1 Concurrency and idempotency

- Updates shall use revision or entity-tag concurrency control.
- Route replacement shall include expected route state.
- Mutating operations shall support correlation/idempotency keys where retries may occur.

### 39.2 Effective configuration

The API shall report separately:

- Draft configuration.
- Active configuration.
- Runtime negotiated/effective format.
- Routed source.
- Effective cue/metadata policy after route override.

### 39.3 Runtime events

Publish events for:

- State and health changes.
- Route add/remove/replace.
- Destination-local restart.
- Peer connect/disconnect.
- Effective format change.
- Backpressure and dropped audio.
- Cue/metadata insertion.
- Required-confirmation failure.
- Device disappearance/recovery.

---

## 40. Observability

### 40.1 Metrics

Expose at minimum:

- Destination state and health.
- Encoder and mux health.
- Bytes and packets sent.
- Effective bitrate.
- Queue depth and duration.
- Buffer over/underflow.
- Dropped media.
- Connection and reconnect count.
- Peer state.
- Required confirmation state.
- Output clock and drift correction.
- Effective latency.
- Cue and metadata success/failure.
- Destination-local restart count.
- Transcode cost signal where practical.

### 40.2 Logs and audit

Logs shall include destination ID, adapter, active revision, route/source ID, operation ID, and correlation ID.

Audit shall include:

- Create/delete.
- Configuration changes.
- Apply/rollback.
- Enable/disable/start/stop.
- Route changes.
- Explicit replacements.
- Test tone and silence operations.
- Authentication-reference changes.
- Cue and metadata policy changes.

---

## 41. Synthetic Destination Validator

The POC shall include a synthetic receiver/validator capable of verifying:

- SRT MPEG-TS output.
- UDP MPEG-TS output.
- RTP/SDP output.
- Codec and format.
- Packet continuity.
- MPEG-TS tables and timing.
- RTP payload/clock/timestamps.
- Silence behavior.
- Route replacement.
- Reconnect behavior.
- Cue timing and insertion.
- Metadata output.
- Output latency.

The validator shall produce machine-readable results for automated tests.

---

## 42. Available POC Test Assets

Expected assets:

- SRT receiver.
- UDP MPEG-TS receiver.
- RTP/SDP receiver.
- HLS web/origin environment.
- ALSA output device.
- AES67 receiver for exploration, not mandatory POC acceptance.
- Synthetic destination validator.

Not currently guaranteed:

- Icecast server.
- Windows output device.
- Provider/downstream cue-accepting system.
- File recording location.

Where an external endpoint is unavailable, the synthetic validator or a controlled test service shall be used. Windows WASAPI and external cue-system demonstrations remain conditional on access to their required platforms/endpoints.

---

## 43. Reference Configuration Examples

### 43.1 SRT MPEG-TS caller

```yaml
destination:
  identity:
    friendlyName: Automation SRT Feed
  lifecycle:
    enabled: true
    autostart: true
  routeBehavior:
    noSource: silence
    sourceFailure: silence-and-retain-route
    transition: hard-cut
  outputFormat:
    sampleRateHz: 48000
    channels: 2
    codec: aac-lc
    bitrateKbps: 192
  transport:
    type: srt
    mode: caller
    remoteHost: automation.example.invalid
    remotePort: 9000
    streamId: publish:station
    passphraseSecretRef: secret-srt-output
    latencyMs: 1000
  payload:
    container: mpegts
    constantBitrate: false
  signaling:
    policy: regenerate
    cueOffsetMs: 0
  recovery:
    initialRetrySeconds: 5
    maxRetrySeconds: 60
    continuousRetry: true
```

### 43.2 UDP MPEG-TS unicast

```yaml
destination:
  identity:
    friendlyName: Receiver UDP Feed
  outputFormat:
    sampleRateHz: 48000
    channels: 2
    codec: L24
  connection:
    interfaceId: stable-interface-id
    localBindAddress: 10.0.0.10
    remoteAddress: 10.0.0.50
    remotePort: 5000
  transport:
    type: udp
    mode: unicast
    packetSize: 1316
  payload:
    container: mpegts
    constantBitrate: true
    muxRateBps: 1500000
    nullPacketPadding: true
```

### 43.3 RTP output

```yaml
destination:
  identity:
    friendlyName: Studio RTP Output
  connection:
    interfaceId: stable-interface-id
    remoteAddress: 239.20.20.20
    remotePort: 5004
  transport:
    type: rtp
    mode: multicast
    payloadType: 96
    ssrc: 12345678
    ttl: 8
  outputFormat:
    codec: L24
    sampleRateHz: 48000
    channels: 2
    packetDurationMs: 1
  sdp:
    downloadable: true
```

### 43.4 HLS hosted output

```yaml
destination:
  identity:
    friendlyName: Public HLS Audio
  transport:
    type: hls
    hostingMode: internal
    targetDirectory: /var/lib/arc/hls/public-audio
    publicBaseUrl: https://audio.example.invalid/public-audio/
  payload:
    mode: standard-hls
    segmentType: mpegts
    segmentDurationSeconds: 6
    playlistWindowSegments: 6
    cleanup: true
    maxRetainedBytes: 1073741824
  outputFormat:
    codec: aac-lc
    sampleRateHz: 48000
    channels: 2
    bitrateKbps: 128
```

---

## 44. Functional Acceptance Criteria

### 44.1 Common configuration and lifecycle

- ARC creates a destination with immutable ID and unique friendly name.
- Classification and tags do not limit routing.
- Draft, validated, active, and running states are distinct.
- Applying a disruptive edit restarts only the affected destination.
- Rollback restores the previous active revision.
- Enabled destinations start with ARC by default.

### 44.2 Routing

- A destination accepts zero or one source.
- Ordinary route addition to an occupied destination is rejected.
- Explicit replace performs a hard cut.
- Compatible replacements preserve encoder/transport session where possible.
- Removing the route applies configured silence/stop behavior.
- Source failure preserves route and outputs silence by default.
- Recovery resumes audio automatically.

### 44.3 Format and processing

- Destinations expose explicit output format.
- Required PCM formats operate.
- Dynamic codec capability reporting is accurate.
- Pass-through policies operate deterministically.
- Incompatible supported routes transcode when policy permits.
- Gain, mute, mapping, conversion, delay, and test tone work as specified.

### 44.4 Network destinations

- SRT caller and listener function with MPEG-TS.
- Listener validates Stream ID and accepts one peer.
- UDP MPEG-TS unicast functions.
- Multicast design fields validate where implemented.
- Constant-bitrate/null-padding selection functions.
- RTP manual output and generated SDP function.

### 44.5 HLS and Icecast

- ARC generates and hosts HLS output when included in the POC build.
- HLS storage limits and cleanup behave safely.
- Standard/LL-HLS capability is reported accurately.
- ARC publishes AAC, MP3, and Opus to an existing Icecast server when a test server is available.
- ICY and Now Playing metadata are sent.

### 44.6 Local devices

- ALSA hardware output operates end to end.
- JACK capability is reported and functions when included.
- WASAPI shared output operates when a Windows endpoint is available.
- Device disappearance preserves route and does not block other pipelines.
- Device recovery resumes output by stable ID.

### 44.7 Health and recovery

- All health dimensions are separately visible.
- Required confirmation level is configurable and enforced.
- Listener Waiting for Peer is non-failed by default.
- A failed destination preserves its route and retries indefinitely.
- One failed/slow destination does not affect other destinations.
- Queues remain bounded.

### 44.8 Cue and metadata

- Destination and route-level policies are resolved correctly.
- Audio delay and cue offset are independent.
- Late cues follow the selected policy.
- Unsupported metadata/cue formats are not selectable.
- Cue insertion and metadata outcomes are observable.

### 44.9 Monitoring, persistence, and licensing

- Post-processing monitor audio is available through authenticated preview.
- One POC monitor-session limit is enforced globally.
- Required configuration, routes, and policies survive restart.
- Runtime peer and statistics state are rebuilt.
- Destination license limits prevent enabling above entitlement.
- Null/test tooling is excluded from licensing.

---

## 45. Required Test Scenarios

1. Create, validate, apply, and start each required destination type.
2. Save an unreachable destination with warnings.
3. Apply a live-safe edit.
4. Apply a disruptive edit and restart only that destination.
5. Roll back the previous revision.
6. Route one source to one destination.
7. Reject a second ordinary source assignment.
8. Explicitly replace the source with a hard cut.
9. Preserve the destination session during compatible replacement.
10. Remove route and transmit silence.
11. Remove route and stop transmission under alternate policy.
12. Continue or suppress signaling during silence according to policy.
13. Route source failure to silence while preserving the route.
14. Automatically restore audio after source recovery.
15. Pass through a compatible source.
16. Force encode a compatible source.
17. Reject incompatible pass-through-only route.
18. Automatically transcode an incompatible supported route.
19. Process stereo output and mono duplication/downmix.
20. Verify independent destination gain and mute.
21. Verify configured destination delay.
22. Verify bounded delay memory.
23. Operate SRT caller MPEG-TS.
24. Operate SRT listener with one peer.
25. Reject mismatched SRT Stream ID.
26. Preserve listener pipeline through peer reconnect.
27. Operate UDP MPEG-TS unicast.
28. Verify UDP packet size and mux settings.
29. Verify configurable CBR and null padding.
30. Operate manual RTP output.
31. Validate generated SDP against active RTP settings.
32. Generate and host HLS when included.
33. Enforce HLS cleanup and storage bounds.
34. Publish each required Icecast codec when endpoint is available.
35. Send ICY/Now Playing metadata.
36. Play to ALSA hardware.
37. Play to JACK when included.
38. Play to WASAPI shared when Windows endpoint is available.
39. Disconnect and recover a local device by stable ID.
40. Fall back to system clock and report Degraded when safe.
41. Fail safely when clock fallback is impossible.
42. Fail destination when required confirmation is absent.
43. Avoid demanding impossible acknowledgment on plain UDP by default.
44. Retry indefinitely with bounded backoff.
45. Keep Waiting for Peer non-failed.
46. Apply backpressure without blocking source or other destinations.
47. Inject, pass, regenerate, translate, and suppress supported cues.
48. Verify cue offset separately from audio delay.
49. Apply late-event policy.
50. Forward or suppress metadata according to effective policy.
51. Run null/test destination without license consumption.
52. Monitor post-processing audio without altering output.
53. Persist destination, route, encoder, cue, and metadata policy.
54. Rebuild peer and runtime state after ARC restart.
55. Enforce enabled-destination license limit.
56. Run four destination tests concurrently and queue additional tests.
57. Verify synthetic validator machine-readable results.
58. Confirm destination failure does not affect sibling destinations.
59. Confirm no destination operation restarts ARC.
60. Audit test-tone authorization and route replacement.

---

## 46. Mandatory End-to-End Demonstrations

The acceptance plan shall include all ten proposed demonstrations:

1. Route an SRT MPEG-TS source to an SRT MPEG-TS destination.
2. Route UDP MPEG-TS input to SRT MPEG-TS output.
3. Route RTP input to RTP output.
4. Route a network source to ALSA output.
5. Route a source to Windows WASAPI output.
6. Automatically transcode an incompatible source for a destination.
7. Replace a destination route without restarting ARC.
8. Maintain silent output when no source is present.
9. Recover a failed destination without affecting other destinations.
10. Insert or forward a test cue while maintaining audio alignment.

Demonstrations 5 and 10 are conditional on access to a supported Windows endpoint and a cue-capable downstream endpoint or synthetic validator. The implementation remains required even when the external endpoint is unavailable; automated validation shall cover it through controlled test infrastructure.

---

## 47. Non-Functional Requirements

- No normal destination action shall require an ARC application restart.
- Destination pipelines shall be isolated.
- Slow or failed destinations shall not block sources or siblings.
- All queues and delay buffers shall be bounded.
- Errors shall not terminate the process.
- Encoder and transport capabilities shall be discovered at runtime.
- Resource consumption shall be measurable for capacity qualification.
- Output timing and cue alignment shall use monotonic internal scheduling and auditable timestamps.
- Secrets shall never appear in ordinary configuration responses, logs, or diagnostics.

---

## 48. AI-Assisted Implementation Guidance

When used in a VS Code AI prompt, the coding assistant shall:

1. Treat this specification as required behavior.
2. Separate route selection, processing, encoding, payloading, transport, signaling, and monitoring interfaces.
3. Build one independently restartable destination pipeline per destination.
4. Never restart ARC for a destination edit or failure.
5. Implement one-source-per-destination validation and explicit atomic replacement.
6. Preserve output sessions across compatible route replacements.
7. Keep queues bounded and isolate backpressure.
8. Generate silence through a defined media path rather than leaving timestamps undefined.
9. Implement deterministic pass-through/force-encode/pass-through-only policies.
10. Rebase cues and metadata to the output timeline.
11. Keep destination audio delay separate from cue offset.
12. Model confirmation levels accurately and never call packets sent playback confirmed.
13. Use secret references and redact sensitive values.
14. Generate protocol-specific validation schemas and capability responses.
15. Include synthetic receivers and automated validators.
16. Include unit, integration, reconnect, timing, backpressure, persistence, and concurrency tests.
17. Mark unavailable platform/protocol features clearly rather than simulating support.
18. Produce API documentation and sample configurations.
19. Instrument all health dimensions with OpenTelemetry-compatible metrics and traces.
20. Maintain compatibility with the source and signal-handling specifications.

---

## 49. Documented Assumptions Requiring Later Review

- File recording is excluded from the POC.
- HLS is hosted directly by ARC, despite earlier recommendation to defer it.
- HLS media-URL authentication requires an explicit security design.
- Icecast server operation is outside ARC; ARC publishes to an existing server.
- UDP unicast is mandatory; multicast remains in the design.
- PCM L16 and L24 are mandatory output codecs; compressed codec delivery depends on applicable adapter/build capabilities.
- RTP clock, packet duration, SSRC, and timestamp fields remain in the schema even though not all were marked explicitly.
- Required downstream confirmation is destination-configurable because UDP cannot provide playback acknowledgment inherently.
- The maximum tested output delay requires capacity qualification.
- Four concurrent destination tests is the POC default.
- One browser monitor is shared across source and destination monitoring.
- Windows and external cue demonstrations depend on endpoint access.
- AES67 remains post-POC despite an available receiver.
- No aggregate bandwidth or encoder ceiling is fixed before performance tests.

---

## 50. Definition of Done

Audio output destination handling is complete for the POC when:

- SRT MPEG-TS, UDP MPEG-TS, RTP, ALSA, WASAPI where testable, and null/test outputs meet this specification.
- Routing cardinality and explicit replacement operate deterministically.
- No-source and source-failure policies operate without route loss.
- Required codecs and conversion paths are validated.
- All destination pipelines restart independently.
- Network binding and protocol-specific configuration are unambiguous.
- HLS and Icecast behavior is implemented according to selected scope and available test infrastructure.
- Clocks, delay, cue offset, and output timeline alignment are observable and tested.
- Health, confirmation, retry, backpressure, and recovery are deterministic.
- Cue/metadata policies operate at destination and route levels.
- Persistence, licensing, monitoring, security, and observability meet this document.
- Synthetic validation covers unavailable or failure-oriented scenarios.
- All mandatory demonstrations are completed directly or through the documented controlled-test substitute when an external endpoint is unavailable.


Supplemental Doc 4:

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


Supplemental Doc 5:

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

Supplemental Doc 6:

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
