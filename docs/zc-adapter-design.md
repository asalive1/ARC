# Zephyr Connect RTP Adapter Design

**Status:** Draft for approval; protocol profile values follow Master Prompt V6
and must not be generalized to unverified provider variants.

## Adapter boundary

Implement `zephyr-connect-rtp` as a provider-specific `SourceAdapter` and
`SignalExtractor`. One explicitly bound UDP socket receives both RTP streams.
Demultiplex by configured SSRC and payload type, never by assuming ordinary
MPEG-TS or a fixed 12-byte RTP header. Audio and GPIO health are independent:
GPIO parser degradation must not stop decodable audio.

## Configuration profile

Persist a versioned provider profile per codec channel:

- stable interface ID, bind address, address family, UDP port, exact allowed
  sender IP and source port;
- audio and GPIO PT/SSRC pair, explicitly configured for each channel;
- AAC-LC/ADTS, expected 48 kHz stereo, bitrate warning band (default 96-160
  kbps), editable decoder delay (default 500 ms), jitter/reorder limits;
- 13-byte GPIO payload, big-endian uint24 mask at bytes 10-12, supported bits
  0-4, active-low conversion, baseline-only initial state, configurable debounce;
- 10 s GPIO heartbeat warning and 15 s stale/unknown defaults;
- timing policy (arrival, RTP timestamp, decoded playout compensation, or
  explicit offset); default decoded-playout compensation plus editable offset.

The port rule is `UDPbasePort + channelIndex`, first channel 9150, through five
channels (9150-9154); validate collisions. V6 provides sample channel SSRCs
`0x00518c3b` audio and `0x00547420` GPIO, but requires configured pairs per
channel. Do not assume those values apply to channels 2-5.

## Receive and parsing stages

1. Reject/count unexpected sender IP/port before accepting a channel.
2. Validate datagram bounds; parse RTP v2, CSRCs, extensions, padding, PT,
   sequence, timestamp, SSRC, and marker with checked offsets.
3. Enforce exactly two configured SSRCs; reject/quarantine unknown or
   conflicting SSRC/PT pairs and count them. Track independent sequence spaces.
4. Maintain bounded per-SSRC reorder/duplicate/loss/jitter accounting and
   bounded queues; report discontinuities.
5. Audio: validate one or more ADTS frames, including CRC-present headers;
   bounded cross-packet reassembly; discard incomplete frames on sequence gap,
   timeout, or overflow; resume at next valid sync. Do not claim CRC validation
   unless actually implemented. Decode through media abstraction; preserve
   RTP 48 kHz shared clock origin and report measured playout delay.
6. GPIO: validate length and configured fixed fields at offsets 1-7; preserve
   all raw bytes and neutral names for fields of unknown meaning. Byte 0 is
   diagnostic only; bytes 8-9 are opaque and excluded from execution,
   deduplication, and timing. Read bytes 10-12 as big-endian uint24, retain full
   mask, and convert active-low bits 0-4 to logical state inside this adapter.
7. Unsupported active bits 5-23 degrade GPIO and publish diagnostics while
   audio continues; never execute an unmapped action.

## Normalized event and GPIO semantics

Emit schema-versioned transport-neutral events with generated ARC event ID,
source ID, signal format/carriage, event type/action, receive time, media
timestamp, RTP sequence, validation status, and attributes including group,
pin, provider bit, logical state, polarity, full raw mask, sender, PT/SSRC, and
preserved provider fields. First valid packet establishes baseline without
transition. Repeated heartbeat refreshes health but emits no duplicate
transition. Preserve state transitions as authoritative; pulse inference adds
duration metadata and never replaces transitions.

On stale timeout retain last value but mark unknown/stale; do not synthesize
ON/OFF. After restart or extended staleness, establish a new baseline before
emitting subsequent transitions. Timing uses arrival and RTP timestamps plus
decoded playout delay and explicit offset; record calculated and actual
dispatch times and expose measured min/max/mean/sample count. POC targets:
synthetic within +/-25 ms; live within +/-50 ms after tuning.

## API/UI and observability

Provide create-disabled-draft, validate, socket probe, apply revision,
enable/start/stop/restart, detected sender/RTP/audio properties, per-pin
state, heartbeat/continuity, recent transitions, and sanitized diagnostics.
Editor exposes interface/bind, port, allowlist, identities, AAC expectations,
mask/polarity/pins, heartbeat, timing, and offsets. Runtime panels expose
packet rates, loss/reorder, decoded format/bitrate, raw/logical mask, per-pin
state, heartbeat, transitions, and separate health dimensions.

## Required fixtures and acceptance

No PCAP is required or implied. Build deterministic synthetic packet fixtures
and a simulator for five concurrent channels. Test RTP CSRC/extensions/padding,
PT/SSRC and sender mismatch, malformed/truncated datagrams, reorder/loss,
duplicate, ADTS multiple frames/CRC/fragments/gaps/bounds, GPIO raw mask and
polarity, baseline/repeated heartbeat/stale/reconnect, reserved bits, timing,
and independent audio/GPIO failure. Final acceptance also needs a live sender,
a real network output plus null/test analysis, and the native Device Server
plugin path; record acceptance environment and measurements before calling it
complete.

## Unresolved protocol constraints

SSRC pairs beyond the supplied channel profile, exact field semantics for
provider bytes 1-7, sender deployment details, and real-device behavior must
be supplied/validated. Do not assert byte-for-byte compatibility beyond the
confirmed profile and live evidence.
