# GitHub Copilot Master Prompt V6

# ARC Audio Routing Controller

## Fully Functional, Elasticity-Ready POC with Zephyr Connect UDP/RTP Integration and Production Evolution Path

**Status:** Draft for review  
**Primary use:** Paste into GitHub Copilot Chat in VS Code as the governing implementation prompt  
**Supersedes:** GitHub Copilot Master Prompt V5, Master Prompt V4, and DRAFT - Audio Routing Controller (ARC) - GitHub Copilot Master Prompt V3  
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
- Elasticity-ready worker, placement, capacity, assignment, reconciliation, and draining contracts.
- A functional Zephyr Connect-compatible UDP/RTP source adapter based on supplied packet evidence.

The POC shall have a production evolution path without requiring redesign of domain models, APIs, route semantics, provider adapters, or media pipeline boundaries.

---

## 3. DOCUMENT PRECEDENCE

Apply requirements in this order:

1. This Master Prompt V6.
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

## 7. ZEPHYR CONNECT DECISIONS AND REMAINING QUESTIONS

The following protocol decisions are approved for the POC and shall be implemented as requirements rather than treated as provisional observations.

### 7.1 Confirmed protocol decisions

1. **SSRC values are fixed per codec channel.**
   - Audio SSRC: `0x00518c3b` for the supplied codec/channel profile.
   - GPIO SSRC: `0x00547420` for the supplied codec/channel profile.
   - Expose these values in the provider profile and diagnostics. Do not permit silent auto-reassignment.

2. **RTP payload types are guaranteed for the POC profile.**
   - Audio payload type: `99`.
   - GPIO payload type: `25`.
   - Packets with conflicting payload type and SSRC combinations shall be rejected or quarantined and counted.

3. **Codec-channel UDP ports follow the Zephyr Connect base-port rule.**
   - `UDP port = UDPbasePort + channelIndex`.
   - Default `UDPbasePort` is `9150`.
   - Define channel indexing explicitly in configuration and UI. The first configured codec channel resolves to port 9150.
   - Validate port collisions across configured sources.

4. **Zephyr Connect POC audio format is AAC-LC with ADTS framing only.**
   - Expected sample rate: 48 kHz.
   - Expected layout: stereo.
   - Support one or more complete ADTS frames per RTP payload.
   - Other Zephyr audio codecs are outside the POC unless separately approved.

5. **GPIO mask format is confirmed.**
   - GPIO payload length: 13 bytes for the approved POC profile.
   - The final three payload bytes, offsets 10 through 12, are a big-endian 24-bit state mask.
   - Five active pins are currently represented by bits 0 through 4.
   - Preserve the complete 24-bit value in diagnostics and the normalized event attributes.

6. **GPIO polarity is active-low.**
   - Provider bit `0` means closed/active and maps to ARC logical `on`.
   - Provider bit `1` means open/idle and maps to ARC logical `off`.
   - Polarity conversion remains inside the Zephyr provider adapter.

7. **GPIO timing alignment shall be configurable.**
   - Preserve RTP timestamp and packet-arrival time for every GPIO packet.
   - Support timing modes for arrival-time dispatch, RTP-timestamp alignment, decoded-audio-playout compensation, and configured offset.
   - POC default: decoded-audio-playout compensation with an explicit configurable offset.
   - Expose calculated and actual dispatch timing for audit and measurement.

8. **No packet capture will be supplied as an automated regression fixture.**
   - Build deterministic synthetic packet fixtures from the approved protocol fields and raw packet examples.
   - Build a Zephyr Connect simulator as mandatory POC test tooling.
   - Do not claim byte-for-byte provider compatibility beyond the confirmed requirements and live acceptance evidence.
   - A future sanitized capture may be added without changing parser or fixture interfaces.

### 7.2 Additional confirmed protocol decisions

The following answers are approved POC requirements.

1. **Exactly two SSRCs appear on one codec-channel UDP port.**
   - One audio SSRC.
   - One GPIO/ancillary SSRC.
   - Additional SSRCs are unexpected and shall be rejected or quarantined and counted.

2. **SSRC values may differ between codec channels.**
   - SSRC values are fixed for each configured codec channel.
   - Configure the audio and GPIO SSRC pair per channel/profile.
   - Do not assume that the channel-1 SSRC pair applies to channels 2 through 5.

3. **The POC shall configure and demonstrate up to five Zephyr Connect codec channels.**
   - Default ports: 9150 through 9154.
   - Each channel has one configured sender IP/source-port pair and one configured audio/GPIO SSRC pair.
   - The POC shall test concurrent operation of five channels through synthetic tooling and demonstrate live operation with the available sender channels.

4. **Sender identity is an exact configured IP and UDP source-port pair.**
   - Packets from another IP or source port are rejected and counted.
   - The sender source port does not change after reconnect.
   - Only one active sender is allowed for one configured codec channel.
   - ARC shall not learn or lock to the first sender dynamically.

5. **RTP CSRC entries, header extensions, and padding may appear.**
   - Parse the complete RTP header.
   - Never assume a fixed 12-byte payload offset.
   - Validate extension and padding lengths before accessing payload data.

6. **ADTS frames may span RTP packets.**
   - Implement bounded cross-packet ADTS reassembly per audio SSRC.
   - Handle partial frame start, continuation, completion, timeout, sequence loss, and buffer overflow.
   - A sequence gap affecting an incomplete ADTS frame shall discard that partial frame and continue at the next valid ADTS sync.

7. **CRC-protected ADTS frames are required.**
   - Support both protection-absent and CRC-present ADTS headers.
   - Validate frame length and header structure.
   - If CRC validation is implemented, expose the result; otherwise preserve the CRC and mark validation capability accurately rather than claiming verification.

8. **Approximately 128 kbps is an expected source characteristic.**
   - Calculate observed bitrate over a rolling interval.
   - Default validation tolerance is configurable.
   - POC default warning range: 96 through 160 kbps.
   - Bitrate outside the warning range degrades expectations but does not alone terminate a valid decodable stream.

9. **Decoder delay is editable.**
   - Default: 500 ms.
   - Expose configured delay, measured buffer fill, effective playout delay, and bounds.
   - Apply decoder/playout delay to GPIO alignment when decoded-audio-playout compensation is selected.

10. **GPIO payload byte 0 is not a mandatory version/type value.**
    - Preserve it as a diagnostic attribute.
    - Do not reject a packet solely because byte 0 differs from `0x19`.

11. **GPIO fields at offsets 1 through 7 shall be validated.**
    - Define their expected values per configured provider profile.
    - Reject or quarantine packets that fail enabled fixed-field validations.
    - Expose each field and validation result in diagnostics.
    - Because the exact semantic names of all fields are not known, use neutral field names such as `providerField1`, `providerField2`, and `gpioGroup` until authoritative names are available.

12. **Bytes 8 through 9 have unknown semantics.**
    - Preserve them as an unsigned big-endian diagnostic value and raw bytes.
    - Do not use them for deduplication, timing, ordering, or state execution unless a later approved specification defines them.
    - Track changes for diagnostic comparison only.

13. **Only GPIO bits 0 through 4 are supported for this POC profile.**
    - Bits 5 through 23 are reserved/unsupported.
    - If a reserved bit becomes active, keep audio running, mark GPIO parsing degraded, preserve the raw mask, publish a diagnostic event, and do not execute an unmapped logical action.

14. **Only one GPIO group appears on one codec channel.**
    - Validate the configured group/port field.
    - Multiple groups on one channel are unsupported for the POC.

15. **Approximately 500 ms GPIO pulses are typical but not guaranteed.**
    - Preserve authoritative ON and OFF state transitions.
    - Calculate observed pulse duration when an ON transition is followed by OFF.
    - Do not require exactly 500 ms for a valid pulse.

16. **Audio and GPIO RTP timestamps share the same 48 kHz clock origin.**
    - Use the shared clock relationship for timestamp-aligned and decoded-playout-compensated modes.
    - Detect and report discontinuities or implausible divergence.

17. **A live Zephyr Connect sender will be available for final POC acceptance.**
    - Synthetic tests remain mandatory.
    - Final provider acceptance shall include a live sender test.

18. **Pin semantics are configurable.**
    - Do not hard-code break-start or break-end to a pin.
    - Map pin state or inferred pulse to normalized semantic events through the mapping engine.

19. **Final GPIO acceptance requires the native Device Server plugin.**
    - Automated tests may use a mock Device Server boundary.
    - Final acceptance shall demonstrate a Zephyr GPIO transition reaching the native Device Server plugin, reconciliation after reconnect, and correct logical GPI state/pulse behavior.

### 7.3 Provisional POC engineering defaults for unresolved tuning values

The following values are POC engineering defaults, not claimed Zephyr protocol guarantees. They shall be configurable, visible in the UI/API, covered by tests, and tuned using live-sender measurements before final acceptance.

#### 7.3.1 RTP loss, reorder, and jitter health

Use RFC 3550-compatible sequence and interarrival-jitter accounting where applicable. RTP itself does not guarantee quality of service, so ARC shall report transport quality and apply configurable application health policy. 

POC defaults:

- Rolling health window: 10 seconds.
- Reorder window: 64 RTP packets per SSRC.
- Duplicate packets: drop and count.
- Late packets outside the reorder window: drop and count.
- Healthy packet loss: less than 0.5 percent in the rolling window.
- Degraded packet loss: 0.5 through 2.0 percent.
- Failed media quality: greater than 2.0 percent for three consecutive rolling windows, or no valid audio packets for 2 seconds.
- Healthy interarrival jitter: less than or equal to 50 ms.
- Degraded jitter: greater than 50 ms and less than or equal to 150 ms.
- Failed timing quality: greater than 150 ms for three consecutive rolling windows.

A media stream that remains decodable may continue while degraded. Health policy shall not silently stop audio solely because a warning threshold was crossed.

#### 7.3.2 Packet-loss concealment

POC behavior:

- Use decoder/GStreamer concealment when the selected decoder exposes a supported mechanism.
- Otherwise discard corrupted or incomplete compressed frames and insert timestamp-correct digital silence for the missing decoded duration when the output pipeline requires continuity.
- Never repeat the last audio frame indefinitely.
- Preserve RTP timing progression.
- Count concealed duration, discarded frames, and silence inserted because of loss.
- Re-synchronize at the next valid ADTS frame boundary.

#### 7.3.3 GPIO heartbeat timeout

The observed heartbeat interval is approximately five seconds. POC defaults:

- Warning after 10 seconds without a valid GPIO packet.
- Stale/unknown after 15 seconds without a valid GPIO packet.
- Retain the last logical value but mark it stale/unknown.
- Do not synthesize OFF or ON transitions on timeout.
- Establish a new baseline after source restart or an extended stale period before publishing later transitions.

#### 7.3.4 Audio/GPIO timing acceptance

POC target:

- For synthetic deterministic tests, calculated GPIO execution relative to decoded-audio playout shall be within plus or minus 25 ms of the configured target.
- For live-sender acceptance, measure and report the offset over repeated transitions. Initial acceptance target is plus or minus 50 ms after configuration/tuning.
- Do not conceal a failed timing target. Report measured minimum, maximum, average, and sample count.
- All timing thresholds and offsets are configurable.

#### 7.3.5 Required real audio destination

Final Zephyr acceptance shall route decoded audio to:

1. The ARC null/test destination for machine-readable analysis; and
2. At least one real network destination supported by the POC, with RTP as the default acceptance destination unless the test plan selects SRT MPEG-TS or UDP MPEG-TS.

The selected real destination shall be documented in `/docs/acceptance-environment.md` before final acceptance.

#### 7.3.6 Reconnect and duration targets

POC provisional targets:

- Detect loss of valid audio packets within 2 seconds.
- Enter reconnect/recovery state without restarting unrelated pipelines.
- Resume valid audio within 10 seconds after the configured sender resumes, excluding external sender startup time.
- Re-establish GPIO baseline before publishing post-reconnect transitions.
- Complete a 60-minute live-sender soak test without process restart, unbounded queue growth, or loss of control-plane availability.
- Complete a five-channel synthetic soak test for 60 minutes.
- Record packet loss, jitter, decoder errors, queue levels, memory growth, GPIO transitions, and timing results.

These are POC acceptance defaults and may be revised by approved product decisions after live measurements.

---

### 7.4 Consolidated approved POC profile

Use configuration and provider profiles, not scattered constants. The approved default profile is:

- UDP base port: `9150`.
- Channel port rule: `UDPbasePort + channelIndex`, with the first channel on 9150.
- RTP version: 2.
- Audio payload type: 99.
- GPIO payload type: 25.
- Audio SSRC: `0x00518c3b`.
- GPIO SSRC: `0x00547420`.
- Audio format: AAC-LC with ADTS framing.
- Audio expectation: 48 kHz stereo.
- GPIO payload length: 13 bytes.
- GPIO mask: offsets 10 through 12, big-endian uint24.
- Enabled GPIO bits: 0 through 4.
- GPIO polarity: active-low.
- Initial GPIO packet: establish baseline without firing transitions.
- Repeated heartbeat state: update health without duplicate transitions.
- GPIO timing: decoded-audio-playout compensation plus configurable offset.
- Maximum POC channel count: five.
- Sender identity: exact configured IP and source-port pair.
- One sender and exactly two SSRCs per channel.
- ADTS cross-packet reassembly: required and bounded.
- CRC-protected ADTS: supported.
- Decoder delay default: 500 ms and editable.
- GPIO heartbeat warning/stale defaults: 10/15 seconds.
- Native Device Server plugin demonstration: required for final GPIO acceptance.
- Missing PCAP strategy: deterministic synthetic fixtures and simulator.

Every profile field shall be represented in one provider-profile schema and surfaced through capability/configuration APIs where appropriate.

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
  allowedSourceAddress: required-ip
  allowedSourcePort: required-port
  sourceIdentityPolicy: exact-ip-and-port
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
  expectedBitrateKbps: 128
  bitrateWarningMinKbps: 96
  bitrateWarningMaxKbps: 160
  expectationPolicy: warn-or-fail-configurable
  decoderDelayMs: 500
  jitterBufferMs: configurable
  adtsCrossPacketReassembly: true
  crcProtectedAdtsSupported: true
gpio:
  payloadLengthBytes: 13
  maskOffsetBytes: 10
  maskSizeBytes: 3
  enabledBits: [0, 1, 2, 3, 4]
  activeLow: true
  initialStatePolicy: baseline-only
  heartbeatWarningMs: 10000
  heartbeatStaleMs: 15000
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

## 20. ELASTICITY-READY POC ARCHITECTURE

The POC will not start or terminate AWS resources. It shall implement the application contracts, state, metrics, and lifecycle behavior required for a future elastic deployment.

### 20.1 Required POC concepts

Implement:

- Worker identity and registration.
- Worker heartbeat and health.
- Platform, architecture, version, and capability reporting.
- Worker capacity reporting.
- Pipeline resource-requirement estimation.
- Capacity-aware worker placement.
- Persisted pipeline assignments.
- Assignment generation numbers.
- A `waiting-for-capacity` operation state.
- Desired-versus-observed assignment reconciliation.
- Worker states for registering, warming, ready, accepting, draining, drained, degraded, failed, and offline.
- Rejection of new assignments while a worker is draining.
- Safe-to-stop determination only after assignments are released.
- Application-specific elasticity metrics.
- Infrastructure-neutral `WorkerScheduler` and `CapacityManager` interfaces.
- Local POC implementations of those interfaces.

### 20.2 Worker model

Expose a worker model equivalent to:

```json
{
  "workerId": "local-worker-1",
  "platform": "linux",
  "architecture": "amd64",
  "version": "0.1.0",
  "state": "ready",
  "drainState": "accepting",
  "capabilities": {},
  "capacity": {},
  "lastHeartbeat": "RFC3339 UTC"
}
```

The POC may have one local worker, but source, destination, and route pipelines shall still be assigned through the scheduler abstraction.

### 20.3 Capability model

Workers shall declare actual capabilities, including:

- Source adapters.
- Destination adapters.
- Codecs and encoders.
- Containers and payload formats.
- Signal parsers and generators.
- Linux or Windows audio backends.
- CPU architecture.
- Hardware acceleration where available.

ARC shall not assign a pipeline to an incompatible worker.

### 20.4 Capacity model

Report real measurements where practical and normalized estimates where necessary:

- CPU utilization and headroom.
- Memory utilization and headroom.
- Network ingress and egress.
- Active source and destination counts.
- Active decoder and encoder counts.
- Delay-buffer memory.
- Queue pressure.
- Preview-session use.
- Available normalized capacity units.

CPU alone shall not determine placement.

### 20.5 Pipeline requirement estimation

Before placement, calculate a bounded, repeatable estimate containing applicable costs for:

- Transport receive/send.
- Demultiplexing.
- Decode.
- Sample-rate/channel conversion.
- Signal extraction.
- Route fan-out.
- Encode and mux.
- Destination delay memory.
- Preview and monitoring.
- Expected network bandwidth.

The POC estimate need not be a final commercial capacity model, but identical inputs shall produce consistent requirements and testable placement outcomes.

### 20.6 Scheduler contracts

Provide interfaces equivalent to:

```rust
trait WorkerScheduler {
    async fn find_placement(
        &self,
        requirements: PipelineRequirements,
    ) -> Result<WorkerPlacement, PlacementError>;
}

trait CapacityManager {
    async fn request_capacity(
        &self,
        requirements: PipelineRequirements,
    ) -> Result<CapacityRequest, CapacityError>;
}
```

The local POC implementations shall:

- Evaluate registered local workers.
- Select a compatible worker with safe capacity.
- Return `waiting-for-capacity` when none is available and policy permits waiting.
- Return a structured rejection when waiting is not permitted.
- Never invoke AWS services.

Future AWS adapters may translate capacity demand and worker-state metrics into ECS and Application Auto Scaling behavior without changing domain models.

### 20.7 Assignment model

Persist assignments equivalent to:

```json
{
  "assignmentId": "assignment-123",
  "resourceType": "route-pipeline",
  "resourceId": "route-123",
  "workerId": "local-worker-1",
  "generation": 4,
  "desiredState": "running",
  "observedState": "running"
}
```

Rules:

- Every replacement assignment receives a higher generation.
- A stale generation shall not become authoritative.
- Desired and observed assignment state are separate.
- Worker loss makes the assignment unowned or recovery-pending without deleting desired route state.
- Reconciliation attempts recovery when compatible capacity is available.

### 20.8 Worker draining

Implement this lifecycle:

```text
accepting
  -> drain-requested
  -> draining
  -> drained
  -> safe-to-stop
```

A draining worker shall:

- Receive no new assignments.
- Publish all remaining assignments.
- Stop, release, or locally reassign eligible pipelines according to policy.
- Remain not-safe-to-stop while assignments remain.
- Publish a final drained state before shutdown.

The POC shall demonstrate draining locally. It does not need to create a replacement AWS task.

### 20.9 Elasticity metrics

Expose OpenTelemetry/Prometheus-compatible metrics including:

- `arc_worker_capacity_utilization_ratio`
- `arc_worker_available_capacity_units`
- `arc_unplaced_pipeline_count`
- `arc_waiting_for_capacity_operations`
- `arc_worker_assignment_count`
- `arc_active_decoder_count`
- `arc_active_encoder_count`
- `arc_network_utilization_ratio`
- `arc_delay_buffer_utilization_ratio`
- `arc_worker_draining`
- `arc_worker_heartbeat_age_seconds`

Metric names may be adjusted to repository conventions, but their semantics shall be documented and stable.

### 20.10 Architecture boundary

Core domain and application code shall not import AWS SDKs or call ECS, CloudWatch, Auto Scaling, EC2, or load-balancer APIs directly.

Use deployment adapters:

```text
ARC domain/application
        |
        v
WorkerScheduler / CapacityManager
        |
        +-- Local POC adapter
        +-- Docker adapter
        +-- Future AWS ECS adapter
```

### 20.11 Elasticity-ready acceptance tests

The POC shall prove:

1. A local worker registers and publishes capabilities.
2. A compatible pipeline is assigned through the scheduler.
3. An incompatible pipeline is rejected with structured reasons.
4. A pipeline waits when compatible capacity is unavailable and waiting is permitted.
5. Worker capacity changes when assignments start and stop.
6. A draining worker receives no new assignments.
7. A worker is not safe to stop while assignments remain.
8. Heartbeat loss changes assignments to recovery-pending or unowned.
9. Reconciliation rebuilds desired pipelines after restart.
10. Capacity, assignment, unplaced-work, and drain metrics are exposed.
11. Assignment generation prevents stale ownership.
12. Architecture tests confirm that domain and application packages do not import AWS SDKs.

---

## 21. OBSERVABILITY

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

## 22. TEST FIXTURES AND SYNTHETIC TOOLS

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

## 23. MANDATORY ZEPHYR TESTS

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
31. Reject a third SSRC on a configured channel.
32. Configure distinct fixed SSRC pairs for five channels.
33. Validate exact sender IP and source-port filtering.
34. Reject a changed sender source port after reconnect.
35. Reassemble one ADTS frame spanning several RTP packets.
36. Discard an incomplete reassembly after sequence loss and recover at the next ADTS sync.
37. Parse CRC-protected ADTS frames.
38. Warn when observed bitrate falls outside the configured range.
39. Validate configured GPIO fields at offsets 1 through 7.
40. Preserve bytes 8 through 9 diagnostically without using them for execution.
41. Report and suppress unsupported GPIO bits 5 through 23.
42. Reject an unexpected GPIO group.
43. Exercise heartbeat warning at 10 seconds and stale state at 15 seconds.
44. Verify shared 48 kHz timestamp alignment and configurable offset.
45. Measure synthetic alignment within plus or minus 25 ms.
46. Demonstrate live alignment within the approved plus or minus 50 ms target after tuning.
47. Route Zephyr audio to null/test and a real RTP destination.
48. Demonstrate Zephyr GPIO through the native Device Server plugin.
49. Run a 60-minute live-sender soak test.
50. Run a five-channel 60-minute synthetic soak test.

---

## 24. INTEGRATED POC ACCEPTANCE DEMONSTRATION

The final POC shall demonstrate this end-to-end sequence:

1. Start PostgreSQL, ARC API, ARC worker, ARC UI, telemetry components, and test dependencies.
2. Create a Zephyr Connect source through the API or UI as a disabled draft.
3. Select a host network interface and UDP port.
4. Validate and activate the configuration.
5. Start the mandatory simulator for automated acceptance and the available live Zephyr Connect sender for final provider acceptance.
6. ARC receives two RTP streams on one UDP port.
7. ARC demultiplexes audio and GPIO.
8. ARC decodes AAC-LC/ADTS audio.
9. ARC displays effective 48 kHz stereo audio properties.
10. ARC establishes GPIO baseline.
11. ARC detects active-low pin state transitions.
12. ARC publishes normalized GPIO events.
13. ARC maps at least one configured pin to a logical GPI action; automated tests may use a mock boundary, but final acceptance uses the native Device Server plugin.
14. ARC routes decoded audio to the null/test validator and at least one real network destination, defaulting to RTP unless another supported destination is approved in the test plan.
15. ARC shows source health, destination health, route state, packet continuity, GPIO state, and audit correlation.
16. ARC restarts.
17. ARC restores the source, destination, mappings, and desired route.
18. ARC re-establishes GPIO baseline without a false transition.
19. ARC resumes audio and subsequent GPIO transition handling.

The POC is not complete if this scenario uses only mocked packet parsing or simulated route state without real UDP reception and actual AAC decode.

---

## 25. DEVELOPMENT PHASES

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

## 26. DEFINITION OF DONE

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
- Local worker registration, capacity-aware placement, waiting-for-capacity, assignment generation, heartbeat-loss recovery, and draining tests pass.
- Core domain and application layers contain no direct AWS SDK dependency.
- All mandatory tests and integrated acceptance demonstrations pass.
- Unanswered protocol questions and production gaps remain explicitly documented.

---

## 27. COPILOT EXECUTION RULES

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
