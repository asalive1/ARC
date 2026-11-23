# Worker Control Protocol

**Status:** Transport decision approved; runtime contract is not yet implemented.

## Decision

API-to-worker control and worker observations use a full-duplex WebSocket
connection carrying versioned JSON messages. Each connection is authenticated
as a worker service identity and bound to that worker's registered `workerId`.
Credential provisioning and rotation remain subject to
[Q13](open-questions.md).

## Versioned envelopes

The shared Rust crate defines the protocol-v1 envelopes:

- `WorkerCommandEnvelope<T>` carries `protocolVersion`, `commandId`,
  `assignmentId`, `generation`, and a typed payload from API to worker.
- `WorkerEventEnvelope<T>` carries `protocolVersion`, `eventId`, `workerId`,
  optional `assignmentId` and `generation`, and a typed payload from worker to
  API.

Fields use camelCase JSON names. Every message must include the supported
protocol version. A peer must explicitly reject an unsupported version; it
must not silently interpret or downgrade it.

## Not frozen by this decision

Phase 1 will define the WebSocket endpoint, connection initiator, handshake
payload, credential mechanism, heartbeat/lease behavior, reconnect policy,
message-size limits, and concrete command/event payload schemas. The Phase 0
API does not expose a worker WebSocket endpoint, and the worker crate is not
yet an executable service.
