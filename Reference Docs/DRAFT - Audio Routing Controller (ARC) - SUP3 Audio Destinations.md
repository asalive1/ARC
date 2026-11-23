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
