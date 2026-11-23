# GitHub Copilot Master Prompt V4

# ARC Audio Routing Controller

## Fully Functional POC with Zephyr Connect UDP/RTP Integration and Production Evolution Path

**Status:** Draft for review  
**Primary use:** Paste into GitHub Copilot Chat in VS Code as the governing implementation prompt  
**Supersedes:** DRAFT - Audio Routing Controller (ARC) - GitHub Copilot Master Prompt V3  
**Required companion inputs:** ARC Supplemental Docs Combined and `zc-protocol-analysis.md`

---

## 0. GOVERNING INSTRUCTION

You are GitHub Copilot acting as the implementation team for ARC.

You shall treat this prompt and the approved supplemental specifications as binding requirements. Do not reinterpret mandatory requirements as suggestions. Do not invent undocumented provider behavior. When a requirement is ambiguous:

1. Record it in `/docs/open-questions.md`.
2. Use an explicit configurable assumption only when this prompt provides a default.
3. Mark that assumption in code, configuration schema, tests, and documentation.
4. Do not claim support for a capability that has not been implemented and tested.

The POC must be runnable, demonstrable, testable, and useful. It is not acceptable to generate only interfaces, empty handlers, mocked media paths, placeholder UI screens, or documentation without an operational end-to-end application.

---

## 1. ROLE

Act as:

- Principal Software Architect
- Broadcast Systems Architect
- Media Systems Engineer
- Rust Backend Engineer
- React/TypeScript Frontend Engineer
- API Architect
- DevOps Engineer
- PostgreSQL Database Architect
- Security Engineer
- QA and Test Automation Engineer
- Network and RTP Engineer
- Audio Transport Engineer
- Technical Writer

ARC is intended for professional radio broadcast, streaming, cloud-hosted radio operations, AoIP environments, GPIO integration, timed signaling, and WideOrbit automation integration.

---

## 2. MISSION

Build a fully functional ARC POC that demonstrates:

- Configurable audio sources.
- Configurable audio destinations.
- Real media transport and audio processing.
- Persistent desired state.
- Audio routing and crosspoint control.
- One-to-many source fan-out.
- Route replacement and recovery.
- Embedded GPIO extraction.
- Normalized control events.
- REST API and real-time events.
- Browser administration.
- Structured logs, metrics, health, and audit.
- Docker deployment.
- Windows and Linux builds.
- Linux AMD64 and ARM64 container builds where dependencies support them.
- Recovery of configured pipelines after restart.
- A functional Zephyr Connect-compatible UDP/RTP source adapter based on supplied packet evidence.

The POC shall have a production evolution path without requiring redesign of domain models, APIs, route semantics, provider adapters, or media pipeline boundaries.

---

## 3. DOCUMENT PRECEDENCE

Apply requirements in this order:

1. This Master Prompt V4.
2. Approved Jira decisions.
3. ARC Supplemental Docs Combined.
4. `zc-protocol-analysis.md` for observed Zephyr Connect behavior.
5. Existing V3 master prompt only where not superseded.

If documents conflict, record the conflict. Do not silently choose one.

---

## 4. REQUIRED PRE-CODE REVIEW GATE

Before creating implementation code, produce these files:

- `/docs/system-architecture.md`
- `/docs/c4-context.md`
- `/docs/c4-container.md`
- `/docs/component-design.md`
- `/docs/media-pipeline-design.md`
- `/docs/zc-adapter-design.md`
- `/docs/database-erd.md`
- `/docs/openapi-plan.md`
- `/docs/security-model.md`
- `/docs/test-strategy.md`
- `/docs/risk-register.md`
- `/docs/assumptions.md`
- `/docs/open-questions.md`
- `/docs/delivery-plan.md`
- `/docs/traceability-matrix.md`

The traceability matrix shall map every mandatory POC requirement to:

- Domain module.
- API endpoint.
- UI surface.
- Test case.
- Acceptance demonstration.
- Current implementation status.

After the documents are generated, continue implementation unless an item is explicitly designated BLOCKING in Section 7.

---

## 5. PRIMARY ARCHITECTURE

Implement ARC as a stateful platform with separation between:

- Durable domain state.
- Control plane.
- Media plane.
- Infrastructure adapters.

### 5.1 Durable state

Persist in PostgreSQL:

- Sources and revisions.
- Destinations and revisions.
- Desired routes.
- Route instances and history.
- Route presets.
- Signal and GPIO mappings.
- Network-interface selections.
- Automation numbers.
- Runtime-operation records.
- Audit records.
- Authentication and API-key metadata.
- Desired enabled/start state.
- Event replay metadata where required.

Do not persist high-frequency audio samples, jitter-buffer contents, decoder frames, encoder frames, socket buffers, or meter samples as desired state.

### 5.2 Control plane

Implement:

- REST API.
- Read-only WebSocket events.
- Authentication and authorization.
- Validation and dry run.
- Revisions and optimistic concurrency.
- Idempotency.
- Operations and cancellation.
- Route controller.
- Desired-versus-observed reconciliation.
- Capability discovery.
- Audit and health.

### 5.3 Media plane

Implement independently managed pipelines for:

- Source receive and demultiplexing.
- Decode or pass-through.
- Normalization and processing.
- Route fan-out.
- Destination encode/mux/transmit.
- Signal extraction and generation.
- GPIO transition detection.

One pipeline failure shall not restart ARC or unrelated pipelines.

---

## 6. TECHNOLOGY STACK

### Backend

Use Rust with:

- Tokio.
- Axum.
- Serde.
- SQLx.
- PostgreSQL.
- `tracing` and OpenTelemetry-compatible instrumentation.
- OpenAPI 3.1 generation.

### Frontend

Use:

- React.
- TypeScript.
- Vite.
- Material UI.
- TanStack Query.
- A virtualized grid library suitable for 64 by 64 crosspoints.

### Media

Primary framework: GStreamer.

Business rules shall not depend directly on GStreamer types. Use media abstraction traits and GStreamer-backed implementations.

### Required interfaces

Define and use abstractions equivalent to:

```rust
trait SourceAdapter;
trait DestinationAdapter;
trait MediaPipeline;
trait SignalExtractor;
trait SignalGenerator;
trait NetworkInterfaceProvider;
trait StateRepository;
trait SecretProvider;
trait EventPublisher;
trait WorkerRegistry;
trait WorkerScheduler;
trait BackupProvider;
```

---

## 7. PRODUCT QUESTIONS REQUIRING ANSWERS

Create these questions in `/docs/open-questions.md`. Questions marked BLOCKING prevent final Zephyr Connect acceptance but do not prevent implementation of configurable behavior and test fixtures.

### 7.1 Zephyr Connect protocol identity

1. **BLOCKING:** Are the audio and GPIO SSRC values fixed per codec, fixed per session, or dynamically selectable?
2. **BLOCKING:** Are payload types 99 for audio and 25 for GPIO guaranteed, configurable, or only observed values?
3. Should ARC identify the streams using:
   - configured SSRC values,
   - configured payload types,
   - automatic payload inspection,
   - or a configurable combination?
4. Can audio and GPIO SSRC values change after reconnect without a configuration change?
5. Can more than two SSRCs appear on the same UDP port?

### 7.2 Channels, ports, and sender validation

6. **BLOCKING:** Does each Zephyr Connect codec channel use `UDPbasePort + channelIndex`, beginning with 9150?
7. What is the highest channel count the POC must configure and test?
8. Must ARC accept packets from any sender, one configured source IP, or a configured IP and source-port pair?
9. Can the sender source port change after reconnect?
10. Is more than one active sender allowed for the same configured input?
11. Should ARC learn and lock to the first valid sender until timeout?

### 7.3 RTP behavior

12. Are standard 12-byte RTP headers guaranteed, or may CSRC entries, RTP extensions, or padding appear?
13. Is RTP version 2 mandatory?
14. What sequence-number discontinuity, reordering, or loss behavior should mark the source degraded?
15. What maximum reorder window should ARC support?
16. Should ARC drop duplicated RTP sequence numbers?
17. Should RTP timestamp discontinuities rebuild only the affected pipeline?

### 7.4 AAC audio behavior

18. **BLOCKING:** Is AAC-LC with ADTS framing the only format required for the Zephyr Connect POC?
19. Can one RTP packet contain one or multiple complete ADTS frames?
20. Can an ADTS frame span RTP packets?
21. Must ARC support CRC-protected ADTS frames?
22. Is 48 kHz stereo mandatory, or must ARC read every ADTS header and support any valid advertised configuration?
23. Is the observed approximately 128 kbps bitrate a validation expectation or only informational?
24. Is the 500 ms decoder-delay configuration a required ARC default, a configurable option, or external-device behavior only?
25. What audio-loss concealment behavior is expected for missing packets?

### 7.5 GPIO payload behavior

26. **BLOCKING:** Confirm that the GPIO state is the final three payload bytes interpreted as a big-endian 24-bit mask, with five active pins currently represented by bits 0 through 4.
27. **BLOCKING:** Confirm that GPIO pins are active-low: bit 0 means closed/active and bit 1 means open/idle.
28. Is byte 0, currently observed as `0x19`, a required version/type value?
29. Must ARC validate the fixed fields at offsets 1 through 7, or expose them only as diagnostics?
30. What is the meaning of the two-byte varying field at offsets 8 through 9?
31. May more than five GPIO pins be represented in the 24-bit mask?
32. Is the GPIO port/group byte configurable, and may several groups appear?
33. Should a changed state be emitted only on transitions, or should heartbeat repeats also be published as semantic events?
34. When the first valid GPIO packet is received, should ARC establish initial state without generating closure events?
35. What heartbeat timeout should mark GPIO state stale or unknown?
36. On heartbeat loss, should states be retained, cleared, or marked unknown?
37. Are approximately 500 ms active-low pulses typical but not guaranteed?
38. Should ARC preserve full state transitions and separately infer pulses?

### 7.6 Audio and GPIO timing

39. **BLOCKING:** Should GPIO transitions be aligned to RTP timestamps, arrival time, decoded audio playout, or a configurable offset?
40. Do audio and GPIO RTP timestamps share the same 48 kHz clock origin?
41. Is the GPIO heartbeat timestamp expected to advance approximately five seconds each packet?
42. What maximum permitted audio/GPIO alignment error must the POC demonstrate?
43. Should the source's configured decoder/jitter delay automatically offset GPIO dispatch?

### 7.7 Required fixtures and acceptance environment

44. **BLOCKING:** Will the packet capture or a sanitized PCAP be provided as an automated regression fixture?
45. Will a live Zephyr Connect sender be available for final POC acceptance?
46. Must ARC generate a simulated Zephyr Connect stream for tests and demos?
47. Which GPIO pins map to the required break-start and break-end semantics?
48. Must GPIO transitions be delivered to WideOrbit Device Server in the POC, or is API/UI event validation sufficient for the first acceptance gate?
49. What destination shall receive the decoded audio in the mandatory Zephyr Connect demonstration?
50. What exact success metrics are required: decode duration, packet-loss tolerance, cue count, timing error, or reconnect time?

### 7.8 Default assumptions until answered

Use configuration, not hard-coded constants. Defaults for the supplied fixture may be:

- UDP port 9150.
- RTP version 2.
- Audio payload type 99.
- GPIO payload type 25.
- Audio SSRC `0x00518c3b`.
- GPIO SSRC `0x00547420`.
- Audio payload contains complete ADTS-framed AAC-LC data.
- Audio expectation is 48 kHz stereo.
- GPIO payload length is 13 bytes.
- GPIO mask occupies payload offsets 10 through 12 as a big-endian 24-bit value.
- Bits 0 through 4 are enabled.
- GPIO polarity is active-low.
- Initial GPIO packet establishes baseline state without firing transitions.
- Repeated heartbeat state does not create duplicate transition events.
- Heartbeat and change packets update last-seen health.

Every default must be editable through the Zephyr Connect source configuration or provider profile.

---

## 8. ZEPHYR CONNECT SOURCE ADAPTER

Implement a provider-specific source adapter named `zephyr-connect-rtp`.

Do not treat this source as ordinary MPEG-TS UDP. It is an RTP-based provider stream with audio and GPIO multiplexed on one UDP socket and differentiated by RTP identity.

### 8.1 Configuration model

Expose fields equivalent to:

```yaml
sourceType: zephyr-connect-rtp
friendlyName: Zephyr Connect 1
network:
  interfaceId: stable-interface-id
  bindAddress: 0.0.0.0
  port: 9150
  addressFamily: ipv4
  allowedSourceAddresses: []
  sourcePortPolicy: any
rtp:
  identificationMode: configured-with-validation
  audioPayloadType: 99
  gpioPayloadType: 25
  audioSsrc: 0x00518c3b
  gpioSsrc: 0x00547420
  reorderWindowPackets: configurable
  duplicatePolicy: drop
audio:
  framing: adts
  codec: aac-lc
  expectedSampleRateHz: 48000
  expectedChannels: 2
  expectationPolicy: warn-or-fail-configurable
  jitterBufferMs: configurable
gpio:
  payloadLengthBytes: 13
  maskOffsetBytes: 10
  maskSizeBytes: 3
  enabledBits: [0, 1, 2, 3, 4]
  activeLow: true
  initialStatePolicy: baseline-only
  heartbeatTimeoutMs: configurable
  staleStatePolicy: mark-unknown
  transitionDebounceMs: configurable
```

### 8.2 Receive and RTP parsing

The adapter shall:

- Bind explicitly to the selected host interface/address.
- Receive audio and GPIO datagrams on one UDP socket.
- Validate packet size before parsing.
- Parse RTP version, payload type, sequence, timestamp, SSRC, marker, CSRC count, extension, and padding correctly.
- Never assume the payload starts at byte 12 without parsing the RTP header.
- Demultiplex using configured and validated RTP identity.
- Track audio and GPIO sequence spaces independently.
- Detect loss, duplicates, reorder, timestamp discontinuity, sender changes, and malformed packets.
- Use bounded queues.
- Preserve sanitized raw fixture references for diagnostics.

### 8.3 AAC/ADTS depayloading

Implement a dedicated depayloading component behind a trait such as:

```rust
trait RtpAudioDepayloader {
    fn push_packet(&mut self, packet: ParsedRtpPacket) -> DepayloadResult;
}
```

The Zephyr implementation shall:

- Locate the RTP payload after complete RTP-header parsing.
- Scan and validate ADTS sync and frame lengths.
- Support multiple complete ADTS frames in one RTP payload.
- Reject malformed and truncated frames safely.
- Report unsupported cross-packet frame fragmentation unless it is explicitly implemented.
- Feed valid ADTS frames into a GStreamer decoder path or another approved decoder abstraction.
- Produce normalized 48 kHz float32 stereo when decoding is required.
- Expose effective codec, sample rate, channels, and bitrate observations.
- Keep audio operational when GPIO parsing fails.

Do not assume `rtpmp4adepay` can parse this provider stream without a verified integration test. A custom RTP payload extractor feeding an ADTS-aware decoder is an acceptable and expected design.

### 8.4 GPIO parsing

Implement a provider parser behind the common `SignalExtractor` and parser contract.

The parser shall:

- Validate configured payload length.
- Preserve all payload fields as provider attributes.
- Read the configured big-endian mask.
- Apply configured enabled-bit range.
- Convert provider electrical polarity into ARC logical state.
- Maintain prior known state per source and GPIO group.
- Establish a baseline on first valid packet.
- Emit state-change events only for changed bits by default.
- Update heartbeat health on every valid GPIO packet.
- Mark state stale/unknown according to configured timeout policy.
- Preserve arrival time, RTP timestamp, RTP sequence, SSRC, payload type, sender address, raw mask, and logical mask.

### 8.5 Normalized GPIO events

Emit transport-neutral events equivalent to:

```json
{
  "schemaVersion": "1.0",
  "arcEventId": "generated-id",
  "sourceId": "source-id",
  "signalFormat": "zephyr-connect-gpio",
  "carriageMethod": "rtp-same-udp-port",
  "eventType": "gpio-state-changed",
  "action": "state",
  "receivedAt": "RFC3339 UTC",
  "mediaTimestamp": 1188073262,
  "sequence": 18753,
  "validationStatus": "valid",
  "attributes": {
    "gpioGroup": 18,
    "pinIndex": 0,
    "providerBit": 0,
    "logicalState": "on",
    "activeLow": true,
    "rawMask": "0x00001e"
  }
}
```

### 8.6 Pulse inference

ARC shall preserve state changes as the authoritative events. Pulse inference may identify an ON followed by OFF and report duration, but it shall not replace the underlying state history.

### 8.7 Health

Expose separate health dimensions:

- UDP socket.
- Sender acceptance.
- Audio RTP continuity.
- Audio decode.
- GPIO RTP continuity.
- GPIO heartbeat.
- GPIO parser.
- Clock/timestamp continuity.
- Overall source health.

Audio may remain Healthy while GPIO is Degraded, and vice versa. Overall health shall show the reason.

---

## 9. ZEPHYR CONNECT API AND UI

### 9.1 API

The source API shall support:

- Create disabled draft.
- Validate configuration.
- Probe socket availability.
- Apply revision.
- Enable/start/stop/restart.
- Read detected sender and RTP identities.
- Read detected AAC/ADTS properties.
- Read current GPIO mask and per-pin state.
- Read heartbeat and packet continuity.
- Read recent GPIO transitions.
- Export sanitized diagnostics.

### 9.2 UI

Provide a Zephyr Connect source editor with:

- Interface and bind address.
- UDP port.
- Sender allowlist policy.
- Identification mode.
- SSRC and payload-type fields.
- AAC expectations.
- GPIO mask offset/size.
- Enabled pins.
- Active-low setting.
- Heartbeat timeout.
- Timing/offset policy.

Provide runtime panels for:

- Sender IP/port.
- Audio and GPIO SSRC.
- Payload types.
- Packet rates.
- Sequence gaps and duplicates.
- Effective audio codec/rate/channels.
- Current raw and logical GPIO mask.
- Per-pin state.
- Last heartbeat.
- Recent transitions.
- Audio/GPIO health.

Unsupported or unverified characteristics must be labeled, not hidden.

---

## 10. SOURCE REQUIREMENTS

Initial POC source adapters:

- Zephyr Connect RTP over UDP with AAC-LC/ADTS and GPIO.
- Generic RTP audio.
- SRT carrying MPEG-TS.
- UDP carrying MPEG-TS.
- HLS.
- Icecast AAC, MP3, and Opus.
- ALSA input where available.
- Windows audio input where available.

Provider-specific Zephyr support is mandatory and shall not be replaced by the generic RTP adapter.

Every source shall support:

- Immutable ID.
- Friendly name.
- Automation number.
- Draft and active revision.
- Explicit network interface.
- Enabled and runtime state.
- Validation and probe.
- Health and statistics.
- Independent restart.
- Persistence and recovery.
- Signal settings.

---

## 11. DESTINATION REQUIREMENTS

Initial POC destination adapters:

- RTP audio.
- SRT carrying MPEG-TS.
- UDP carrying MPEG-TS.
- ALSA output.
- Windows WASAPI shared output.
- Null/test destination.

Design and capability reporting shall also include HLS and Icecast publishing where implemented.

Each destination owns an output contract and supports:

- Prefer pass-through, force encode, or pass-through only.
- Explicit output format.
- Gain and mute.
- Channel conversion.
- Sample-rate conversion.
- Configurable destination delay.
- No-source silence or stop.
- Source-failure behavior.
- Independent retry and recovery.
- Cue/metadata policy.

---

## 12. ROUTING REQUIREMENTS

Rules:

- Any compatible source may route to any compatible destination.
- One destination has zero or one desired source.
- One destination has zero or one active source.
- One source may feed many destinations.
- Mixing is not supported.
- Independent left/right routing is not supported.

Implement:

- Immediate and Take modes.
- Explicit atomic route replacement.
- Fan-out.
- Atomic and best-effort batches.
- Presets/salvos.
- Undo and rollback.
- Locks and permissions.
- Desired and active route state.
- Route revisions and global routing sequence.
- 64 by 64 virtualized matrix.
- Real-time multi-user synchronization.

A source signal policy may differ for each routed destination.

---

## 13. GPIO AND SIGNAL REQUIREMENTS

### 13.1 Logical model

Keep provider electrical representation inside the provider adapter. The ARC event bus and Device Server boundary use logical state.

### 13.2 GPI

Support:

- State.
- Pulse representation.
- Debounce.
- Mapping.
- Stale/unknown state.
- Requested versus observed timing.

### 13.3 GPO

Support:

- High/on.
- Low/off.
- Pulse.
- Mapping to output signal profiles.

### 13.4 Mapping

Zephyr GPIO pin changes shall be mappable to:

- Logical Device Server GPI.
- UI event.
- Audit only.
- API/WebSocket event.
- Route/destination signal behavior where configured.

Do not hard-code pin 0 as break start or pin 1 as break end. Those are mapping configuration decisions.

---

## 14. DEVICE SERVER INTEGRATION

Maintain a separately deployable `/device-server-plugin` project.

The plugin shall support:

- Logical GPI and GPO.
- Audio crosspoints.
- State snapshots.
- Reconnect and reconciliation.
- Versioned protocol.
- Health.

Preferred POC ARC-to-plugin protocol: authenticated versioned WebSocket JSON.

Tests shall use a mock Device Server boundary plus a separate native-plugin build path.

---

## 15. AUTOMATION COMMAND LISTENER

Maintain the V3 automation listener as a separate first-class subsystem, not an HTTP API replacement.

Support:

- TCP and optional TLS listeners.
- API-key validation.
- Source IP filtering.
- Route commands by automation number or exact case-insensitive friendly name.
- GPO high, low, and pulse commands.
- Immediate execution.
- Audit and rate limits.

Do not couple the Zephyr Connect UDP/RTP receiver to the automation command listener.

---

## 16. API REQUIREMENTS

Use `/api/v1` REST/JSON and a read-only WebSocket event channel.

Implement:

- OpenAPI 3.1.
- Interactive docs.
- TypeScript SDK.
- C# SDK generation assumption.
- ETag and If-Match.
- Explicit revisions.
- JSON Merge Patch and PUT.
- Idempotency keys.
- Asynchronous operations.
- RFC 9457-compatible problem details.
- Cursor pagination.
- Filtering, sorting, and bounded expansion.
- Bootstrap snapshot and event replay.
- Basic authentication and API keys for the POC.
- Secret references without reveal.
- Rate and request-size limits.
- Backup and restore.

Endpoint families shall cover sources, destinations, routes, presets, mappings, signals, GPIO, operations, health, statistics, capabilities, audit, licensing, authentication, keys, secrets, system, backups, restores, and bootstrap.

---

## 17. NETWORK INTERFACES

Network interfaces are first-class resources.

Discover and expose:

- Stable interface ID.
- Name.
- IPv4 and IPv6 addresses.
- MAC address where available.
- MTU.
- Link state.
- Administrative state.
- Multicast capability.
- Link speed where available.

Every network source and destination shall explicitly select an interface or valid bind address.

Linux Docker media workers should support host networking where required for correct interface and multicast behavior. Document platform limitations.

---

## 18. STATEFULNESS AND RECOVERY

Persist desired state in PostgreSQL only. Do not support SQLite as the normal POC database.

After restart:

1. Load active source and destination revisions.
2. Load desired routes.
3. Recreate enabled pipelines.
4. Reconcile desired and observed state.
5. Restore route state.
6. Publish recovery transitions.
7. Do not replay expired GPIO or cue events unless configured.

The Zephyr adapter shall re-establish baseline GPIO state after restart before publishing new transitions.

---

## 19. WORKER AND DEPLOYMENT ARCHITECTURE

Implement logical API and worker services even when colocated.

### API service

- REST.
- WebSocket.
- Authentication.
- Configuration.
- Routing commands.
- Audit and operations.

### Worker service

- Network interface discovery.
- Source and destination pipelines.
- Media routing.
- Zephyr adapter.
- Signals and GPIO.
- Health and capacity.

The POC may deploy them on one host, but use versioned internal contracts and shared domain services.

Support build targets:

- Linux x86-64.
- Linux ARM64.
- Windows x86-64.

Capability reporting must distinguish unavailable codecs, backends, and adapters by platform.

---

## 20. OBSERVABILITY

Implement from the first vertical slice:

- Structured logs.
- OpenTelemetry traces.
- Prometheus-compatible metrics.
- Health endpoints.
- Audit records.
- Correlation IDs.

Zephyr metrics shall include:

- Audio packets received.
- GPIO packets received.
- Packet rate per SSRC.
- Sequence gaps.
- Duplicates.
- Reordered packets.
- Malformed RTP.
- Malformed ADTS.
- Parsed ADTS frames.
- GPIO mask changes.
- Heartbeat age.
- GPIO stale transitions.
- Audio and GPIO timestamp discontinuities.
- Decode and parser failures.

---

## 21. TEST FIXTURES AND SYNTHETIC TOOLS

Create `/test-fixtures/zephyr-connect` containing supplied or sanitized fixtures when available.

Implement a Zephyr Connect simulator capable of:

- Sending audio and GPIO RTP to the same UDP port.
- Configurable audio/GPIO SSRC and payload types.
- ADTS AAC fixture playback.
- Multiple ADTS frames per RTP payload.
- Configurable packet loss, duplicate, reorder, jitter, and timestamp discontinuity.
- GPIO heartbeat packets.
- Active-low state changes.
- Configurable pulse duration.
- Malformed payload injection.
- Sender IP/port change simulation where test infrastructure permits.

The simulator is test tooling and shall not be represented as the actual provider protocol authority.

---

## 22. MANDATORY ZEPHYR TESTS

Implement automated tests for:

1. RTP audio and GPIO demultiplexing on one UDP port.
2. Configured SSRC and payload-type matching.
3. Rejection or quarantine of unknown streams.
4. RTP header with CSRC entries.
5. RTP header extension.
6. RTP padding.
7. Sequence wraparound.
8. Packet loss.
9. Duplicate packets.
10. Reordered packets.
11. RTP timestamp discontinuity.
12. One ADTS frame per packet.
13. Multiple ADTS frames per packet.
14. Malformed ADTS sync.
15. Truncated ADTS frame.
16. AAC decode to 48 kHz stereo PCM.
17. First GPIO packet establishes baseline.
18. GPIO 0x1F to 0x1E produces pin 0 logical ON for active-low configuration.
19. GPIO 0x1E to 0x1F produces pin 0 logical OFF.
20. GPIO 0x1F to 0x1D produces pin 1 logical ON.
21. Repeated heartbeat with unchanged mask emits no duplicate transition.
22. Heartbeat timeout marks GPIO stale/unknown.
23. Audio remains operational during malformed GPIO.
24. GPIO remains observable during audio decode failure.
25. Source restart re-establishes baseline without false trigger.
26. Event mapping routes a selected pin to logical Device Server GPI.
27. WebSocket publishes source and GPIO health changes.
28. Audit correlates packet, transition, mapping, and downstream action.
29. Route decoded audio to null/test destination.
30. Route decoded audio to a real required destination.

---

## 23. INTEGRATED POC ACCEPTANCE DEMONSTRATION

The final POC shall demonstrate this end-to-end sequence:

1. Start PostgreSQL, ARC API, ARC worker, ARC UI, telemetry components, and test dependencies.
2. Create a Zephyr Connect source through the API or UI as a disabled draft.
3. Select a host network interface and UDP port.
4. Validate and activate the configuration.
5. Start a supplied live stream, replay fixture, or simulator.
6. ARC receives two RTP streams on one UDP port.
7. ARC demultiplexes audio and GPIO.
8. ARC decodes AAC-LC/ADTS audio.
9. ARC displays effective 48 kHz stereo audio properties.
10. ARC establishes GPIO baseline.
11. ARC detects active-low pin state transitions.
12. ARC publishes normalized GPIO events.
13. ARC maps at least one configured pin to a logical GPI action or approved mock boundary.
14. ARC routes decoded audio to at least one real destination and the null/test validator.
15. ARC shows source health, destination health, route state, packet continuity, GPIO state, and audit correlation.
16. ARC restarts.
17. ARC restores the source, destination, mappings, and desired route.
18. ARC re-establishes GPIO baseline without a false transition.
19. ARC resumes audio and subsequent GPIO transition handling.

The POC is not complete if this scenario uses only mocked packet parsing or simulated route state without real UDP reception and actual AAC decode.

---

## 24. DEVELOPMENT PHASES

### Phase 0: Architecture and contracts

- Required documents.
- Workspace/repository structure.
- Database schema and migrations.
- OpenAPI skeleton.
- Media abstractions.
- Capability model.
- Test strategy and fixtures.

### Phase 1: Stateful vertical audio slice

- PostgreSQL persistence.
- Source and destination lifecycle.
- Worker lifecycle.
- Generic RTP source.
- RTP and null/test destinations.
- One route.
- Health and WebSocket events.
- Restart recovery.

### Phase 2: Zephyr Connect adapter

- Same-port RTP demultiplexing.
- ADTS depayloading and AAC decode.
- GPIO parser and baseline/state transitions.
- Zephyr UI and API fields.
- Simulator and regression tests.
- End-to-end audio plus GPIO demo.

### Phase 3: Routing expansion

- 64 by 64 matrix.
- Replacement.
- Fan-out.
- Batches.
- Presets.
- Undo/rollback.
- Locks and concurrency.

### Phase 4: Transport expansion

- SRT MPEG-TS input/output.
- UDP MPEG-TS input/output.
- HLS and Icecast as selected.
- ALSA and WASAPI platform adapters.

### Phase 5: Signals and integrations

- Normalized signal engine.
- Device Server mock and native plugin path.
- Automation command listener.
- GPO generation.
- SCTE-35 or additional formats selected for POC.

### Phase 6: Hardening

- Backup/restore.
- Security and permission tests.
- Fault injection.
- Performance tests.
- ARM64 and Windows build validation.
- Installation and operations documentation.

Each phase must end with a runnable application and passing tests. Do not postpone integration until the final phase.

---

## 25. DEFINITION OF DONE

ARC POC is complete only when:

- Source, destination, route, signal, API, UI, and persistence layers operate through shared domain services.
- Real UDP packets are received through a selected interface.
- The Zephyr Connect provider adapter demultiplexes audio and GPIO on one UDP port.
- AAC-LC/ADTS audio is actually decoded and routed.
- GPIO masks are parsed, polarity is applied, baseline is established, and transitions are emitted.
- Provider-specific values are configurable and validated.
- Desired state survives restart.
- Pipelines rebuild without restarting the whole application.
- The required routing matrix and operations function.
- REST and WebSocket contracts are documented and tested.
- Structured logs, metrics, traces, health, and audit are operational.
- Docker deployment succeeds on Ubuntu 24.04 AMD64 and ARM64 where dependencies permit.
- Windows x86-64 build succeeds and platform limitations are documented.
- PostgreSQL migrations and backup/restore function.
- Synthetic Zephyr and destination validators produce machine-readable results.
- All mandatory tests and integrated acceptance demonstrations pass.
- Unanswered protocol questions and production gaps remain explicitly documented.

---

## 26. COPILOT EXECUTION RULES

1. Plan before generating large code changes.
2. Implement one vertical slice at a time.
3. Run formatting, lint, unit tests, integration tests, and contract validation after each slice.
4. Do not generate placeholder implementations for mandatory acceptance behavior.
5. Do not label mocks as completed provider support.
6. Do not hard-code observed SSRC, payload type, pin semantics, or port beyond editable defaults.
7. Preserve actual packet fixtures and parser regression tests.
8. Reject malformed input safely and keep queues bounded.
9. Keep audio operational when signal parsing fails unless policy says otherwise.
10. Keep signals observable when audio decode fails.
11. Never expose secrets or raw stack traces through the API.
12. Keep AWS-specific SDKs out of domain and application layers.
13. Update the traceability matrix as work is completed.
14. Maintain `/docs/known-limitations.md` throughout implementation.
15. At the end of every phase, provide exact run commands and a demonstration checklist.
