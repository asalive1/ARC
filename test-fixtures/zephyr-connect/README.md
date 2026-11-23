# Zephyr Connect Synthetic Fixtures

No packet capture is supplied as a regression fixture. Phase 0 reserves this
directory for deterministic, sanitized inputs and simulator definitions. The
test generator must create audio and GPIO RTP streams on one UDP port and must
allow per-channel SSRC/PT values, sender identity, ADTS frame layout, packet
loss/reorder/jitter, timestamp discontinuity, GPIO mask transitions, and
malformed-packet cases to be configured.

Do not check in live sender addresses, credentials, or unsanitized capture data.
Synthetic fixtures demonstrate parser behavior against the approved contract;
they do not establish undocumented provider protocol semantics.
