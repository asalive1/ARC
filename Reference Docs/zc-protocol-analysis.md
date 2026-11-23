# Zephyr Connect Stream Protocol Analysis

> **Date:** 2026-04-16
> **Source:** Packet capture from 10.29.1.91 port 9150 (10 minutes, 28,201 packets, 12MB)
> **ZC TX source:** 99.49.96.47:55097 → 10.29.1.91:9150 (UDP)

---

## Summary

The Zephyr Connect codec stream consists of **two RTP streams multiplexed on the same UDP port**, differentiated by SSRC:

| Stream | SSRC | Packets | Rate | Purpose |
|--------|------|---------|------|---------|
| Audio | `0x00518c3b` | 28,079 | ~47 pkt/s (~21ms interval) | AAC-LC encoded audio |
| GPIO/Ancillary | `0x00547420` | 122 | ~0.2 pkt/s (~5s interval) | GPIO state + ancillary data |

---

## Audio Stream

### RTP Header
- **Payload Type:** 99 (dynamic)
- **SSRC:** `0x00518c3b`
- **Marker bit:** 0 (never set)
- **Timestamp increment:** 1024 per packet (= 1024 samples at 48kHz = 21.33ms)

### Codec: AAC-LC with ADTS Framing
- Payload begins with ADTS sync word `0xFFF`
- ADTS header decode of `0xFFF94C80`:
  - Sync: `0xFFF`
  - ID: 0 (MPEG-4)
  - Layer: 0
  - Protection absent: 1 (no CRC)
  - Profile: 1 (AAC-LC, objectType=2)
  - Sampling frequency index: 3 (48000 Hz)
  - Channel configuration: 2 (stereo)
- **Variable bitrate** — packet sizes range from 320-460 bytes (payload), averaging ~340 bytes
- Estimated bitrate: ~128 kbps average

### Multiple ADTS Frames Per Packet
The RTP payload type field shows values like `99,127,40` — tshark is decoding the CSRC count or extension. The payload contains multiple ADTS frames concatenated (each starting with `0xFFF`).

---

## GPIO/Ancillary Stream

### RTP Header
- **Payload Type:** 25 (dynamic)
- **SSRC:** `0x00547420`
- **Timestamp increment:** 240,640 per packet (= 240,640 samples at 48kHz = ~5.013s)
- **Packet interval:** ~5.013 seconds (periodic heartbeat)

### Payload Structure (13 bytes)

```
Offset  Size  Field              Example    Notes
------  ----  -----              -------    -----
0       1     Header/Version     0x19       Fixed value (25 decimal = matches PT 25?)
1       4     Sequence/ID        00000001   Fixed — possibly channel/codec ID
5       2     Unknown            0034       Fixed — possibly config/type identifier
7       1     GPIO port/group    12         Fixed — GPIO port identifier (18 decimal)
8       2     Varying field      7d04       Changes every packet — possibly timestamp/counter
10      3     GPIO state mask    00001f     GPIO pin states as bitmask
```

### GPIO State Bitmask (bytes 10-12, big-endian uint24 or last byte significant)

The last byte is the GPIO state bitmask. Observed values:

| Value | Binary | Meaning |
|-------|--------|---------|
| `0x1F` | `00011111` | All 5 GPIO pins HIGH (idle/normal state) |
| `0x1E` | `00011110` | Pin 0 LOW, pins 1-4 HIGH (GPIO trigger!) |
| `0x1D` | `00011101` | Pin 1 LOW, pins 0,2-4 HIGH (GPIO trigger!) |

### GPIO State Transitions Observed

| Time (relative) | State | Event |
|-----------------|-------|-------|
| 3.69s | `0x1F` | Initial state — all pins high |
| 215.10s (~3:35) | `0x1E` | **Pin 0 went LOW** (contact closure) |
| 215.59s (~3:36) | `0x1F` | Pin 0 returned HIGH (pulse ~490ms) |
| 335.49s (~5:35) | `0x1D` | **Pin 1 went LOW** (contact closure) |
| 336.00s (~5:36) | `0x1F` | Pin 1 returned HIGH (pulse ~510ms) |

These are classic satellite break triggers — momentary contact closures (~500ms pulses) on individual GPIO pins.

### Interpretation

- **5 GPIO pins** are being transported (bits 0-4 of the state byte)
- Pins are **active-low** (closure = 0, idle = 1)
- The GPIO stream sends periodic heartbeats (~5s) with current state
- State changes are sent immediately (not just on the 5s heartbeat)
- The varying 2-byte field at offset 8 appears to be a sub-counter or timestamp within the GPIO stream

---

## Protocol Summary for Implementation

### To receive and decode this stream:

1. **Listen on UDP port** (e.g., 9150)
2. **Parse RTP header** — extract SSRC to demux audio vs GPIO
3. **Audio (SSRC `0x00518c3b`):**
   - Strip 12-byte RTP header
   - Feed ADTS-framed AAC payload to decoder (FFmpeg/GStreamer `avdec_aac` or `fdkaacdec`)
   - Output PCM 48kHz stereo
4. **GPIO (SSRC `0x00547420`):**
   - Strip 12-byte RTP header
   - Read byte at offset 10+2 (or last byte of payload) as GPIO bitmask
   - Compare with previous state to detect transitions
   - Fire `gpio-inputs-changed` events on state change

### GStreamer Pipeline (conceptual):

```
udpsrc port=9150
  → rtpssrcdemux          # Split by SSRC
  
  SSRC audio → rtpmp4adepay or custom depay → avdec_aac → audioconvert → output
  SSRC gpio  → appsink    # Custom Python handler for GPIO parsing
```

Note: Since the AAC is ADTS-framed in RTP with a dynamic payload type, GStreamer's standard `rtpmp4adepay` may not work directly. May need a custom depayloader or use `rtpgstdepay`/raw `appsink` to strip RTP and feed ADTS to the decoder.

---

## Raw Packet Examples

### Audio Packet
```
RTP: V=2, PT=99, Seq=34064, TS=1187896110, SSRC=0x00518c3b
Payload (332 bytes): fff94c802801b42139c8000001fff90a5c3d042b4ae6...
                     ^^^^ ADTS sync word
```

### GPIO Packet
```
RTP: V=2, PT=25, Seq=18711, TS=1188073262, SSRC=0x00547420
Payload (13 bytes): 19 00000001 0034 12 7d04 0000 1f
                    ^  ^^^^^^^^ ^^^^ ^^ ^^^^ ^^^^ ^^
                    |  chan/id  type port cntr      GPIO bitmask (5 pins all HIGH)
```

### GPIO Packet (trigger active)
```
RTP: V=2, PT=25, Seq=18753, TS=..., SSRC=0x00547420
Payload (13 bytes): 19 00000001 0034 12 1b04 0000 1e
                                                   ^^
                                                   0x1E = 0b00011110 = pin 0 LOW
```

---

## ZC Configuration (from live system)

Verified from `/opt/telos/zephyr-connect-WO-TEST/ZephConnConfig/` on 10.29.1.91.

### Codec Configuration (`zephier16.cfg`)
```ini
[codec_1]
ENCType = ZEPH_AAC_LC
BitRate = ZEPH_BRATE_128
SampleRate = ZEPH_SRATE_48000
Mode = ZEPH_STEREO
protocolTypeDec = UDP
DecDelay = 500                    # 500ms decoder delay
GPIO_port1_op_mode_ENC = ENDPOINT # GPIO embedded in codec stream
GPIO_port1_protocol_type_ENC = None
Enable = 2
EnableEncoder = 2
EnableDecoder = 2
```

### Global Options (`zephyrGlobalOptions.cfg`)
```ini
UDPbasePort = 9150       # Codec 1 = 9150, Codec 2 = 9151, etc.
TCPbasePort = 9550
count = 1                # 1 active codec channel
LW_audio_fault_timeout_ms = 2000
```

### Key Findings from Config
- GPIO mode is ENDPOINT with protocol "None" — GPIO travels embedded in the codec stream, NOT via separate LiveWire GPIO multicast
- All 3 data channels are disabled (`enable = 0`) — no sideband data
- LWCP keeps reconnecting because there's no LiveWire network on the cloud host (expected)
- 64 codec channels licensed but only 1 active
- Decoder pre-buffer: 500ms configured, ~180ms measured

### DSP Periodic Log (runtime stats)
```
DECODER::Accepted IP  A  99.49.96.47
DECODER::Accepted sorce port A = 55097
DECODER::Dec stream params: codecID = 2 samplerateID = 48000 bitrateID = 128000 channelID = 2
DECODER::SNMP jitter level A = 13.0
DECODER::PRE Buffer_FILL = 180.6
DECODER::num pakets = 469
```
