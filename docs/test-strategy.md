# Test Strategy

**Status:** Draft for approval; tests are planned, not currently implemented.

## Test levels

1. **Domain/unit:** route cardinality, revisions, state transitions, mapping
   policy, active-low conversion, operation states, placement estimates,
   generations, leases, drain safety, idempotency decisions.
2. **Adapter/component:** PostgreSQL repositories/migrations; Axum request and
   problem details; interface discovery; GStreamer adapter capabilities;
   RTP/ADTS and GPIO parsing; Device Server protocol translation.
3. **Integration:** PostgreSQL-backed API/application tests; local worker and
   scheduler; operation/reconciliation lifecycle; API<->worker messages;
   authenticated plugin mock; read-only WebSocket events and replay/snapshot.
4. **UI:** browser workflows for source/destination setup, explicit interface
   selection, validate/apply, routing matrix, take/replace/fan-out, operation
   progress, health, GPIO state, concurrency conflict, reconnect and
   permissions.
5. **Media/E2E:** generated RTP -> ARC -> RTP; Zephyr simulator -> decode/route ->
   null/test and real network output; packet loss/reorder; independent branch
   failure; restart recovery.
6. **Deployment/platform:** Compose startup/health/shutdown and PostgreSQL
   persistence; Ubuntu 24.04 AMD64/ARM64; Windows x86-64 build and documented
   adapter limitations; license inventory and dependency compatibility.
7. **Resilience/performance:** restart, worker heartbeat loss, stale
   assignment, capacity shortage, drain, PostgreSQL interruption, queue bounds,
   soak, malformed traffic, failure isolation and memory/resource trends.

## Required fixtures and deterministic tests

Build synthetic RTP/ADTS/GPIO packets and a Zephyr simulator; no packet capture
is assumed. Cover RTP CSRC/extension/padding, sender/PT/SSRC mismatch, sequence
wrap/gaps/reorder/duplicates, timestamps, malformed/truncated bounds,
multiple ADTS frames, CRC-present frames, cross-packet reassembly/timeout/
overflow, decoder loss behavior, full GPIO mask/polarity, baseline, heartbeat,
stale/restart, reserved bits, and audio/GPIO failure isolation. Explicit Zephyr
cases include five distinct channel identity pairs, third-SSRC rejection, exact
sender IP/port and changed-port rejection, provider-field validation at offsets
1-7, diagnostic-only byte 0 and bytes 8-9, unexpected group, bitrate warning,
heartbeat warning/stale thresholds, shared 48 kHz clock alignment, and decoded
48 kHz stereo output. Verify the active-low transitions `0x1F -> 0x1E` (pin 0
ON), `0x1E -> 0x1F` (pin 0 OFF), and `0x1F -> 0x1D` (pin 1 ON).

The simulator must send both streams to one UDP port and configure PT/SSRC,
play ADTS fixtures, send multiple ADTS frames per packet, vary loss/duplicate/
reorder/jitter/timestamps, send GPIO heartbeat and active-low changes with
configurable pulse durations, inject malformed payloads, and support sender
identity-change scenarios where test infrastructure allows. It is test tooling,
not an authority for unconfirmed provider behavior.

Use disposable PostgreSQL instances matching supported major versions for
migration-upgrade, repository constraints, rollback, and persistence tests.
Exercise API idempotency and `If-Match` conflicts and verify UI uses the same
domain rules as external clients.

## Required acceptance runs

- Generic RTP source through worker to RTP destination and null/test sink.
- Five concurrent synthetic Zephyr channels for a 60-minute soak.
- Live Zephyr sender for 60 minutes; detect packet loss within 2 s and recover
  valid media within 10 s after sender resumes (excluding sender startup).
- Synthetic GPIO timing within +/-25 ms; live timing measured and targeted within
  +/-50 ms after tuning, with min/max/average/sample count reported.
- GPIO transition through normalized mapping to native Device Server plugin;
  verify reconnect snapshot/baseline and logical state/pulse behavior.
- Docker on Ubuntu 24.04 AMD64 and ARM64 where media dependencies permit;
  verify desired-state and route recovery after restart.
- Scheduler: compatible placement, structured incompatibility, waiting for
  capacity, changing capacity, drain refusal, safe-to-stop, heartbeat loss,
  reassignment generation fencing, and restart reconciliation.
- Zephyr observability: per-SSRC packet rate; audio/GPIO packets; gaps,
  duplicates, reorder, malformed RTP/ADTS, parsed frames, GPIO mask changes,
  heartbeat age/stale transitions, timestamp discontinuity, decode/parser
  failures; correlate packet, transition, mapping, and downstream action in
  audit.
- Destination behavior: pass-through/encode policy, format, gain/mute,
  channel/sample-rate conversion, delay, no-source silence/stop, source-failure
  behavior, retry/recovery, and cue/metadata policy.
- Full integrated recovery: restart API/worker; restore source, destination,
  mappings and desired route; re-establish GPIO baseline without a false
  transition; resume audio and subsequent GPIO event handling. Mock-only packet
  parsing or simulated route state does not satisfy this gate.

## Quality gates

Every phase adds automated unit, integration, API, UI, and applicable media,
recovery, and deployment tests. Test expectations are tied to the
[traceability matrix](traceability-matrix.md). No phase is complete on a green
unit suite alone; run the phase's documented end-to-end/demo gates.
