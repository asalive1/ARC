# Device Server Protocol v1

The native Device Server plugin is independently built and deployed. This
directory defines its versioned wire contract only; it has no database, ARC
media, or direct ARC implementation dependency.

The approved POC transport is authenticated WebSocket JSON. The first protocol
cut must define capability and state snapshots, logical GPI/GPO state, pulse
requests, crosspoint operations, correlation IDs, reconnect/reconciliation,
health, and protocol negotiation. Mutating commands require idempotent command
IDs and structured outcomes. Provider electrical polarity is translated at the
adapter boundary; protocol state is logical on/off.

This scaffold intentionally does not invent the native plugin ABI, vendor
message vocabulary, or authentication mechanism. Resolve those in
[the open questions](../../../docs/open-questions.md) before native plugin
implementation.
