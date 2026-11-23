# Security Model

**Status:** Approved architecture baseline; threat model and implementation details require
security review before deployment.

## Trust boundaries and controls

| Boundary | Required controls |
|---|---|
| Browser/integrator -> API | TLS in deployed environments; POC username/password and API keys; bounded requests/rate limits; CSRF-safe browser auth design; generic external errors |
| Principal -> operation | Central authorization for UI/API/automation/plugin paths; least-privilege scopes; auditable decisions; locks and revisions do not replace authorization |
| API -> worker | Authenticated service identity, protocol version, message size limits, generation fencing, capability validation, replay/idempotency handling |
| ARC -> Device Server plugin | Independently provisioned credentials, authenticated WebSocket JSON, version negotiation, allowlisted commands, reconnect/resnapshot, audit |
| Worker -> network/device | Explicit interface/bind and device permissions; sender allowlists; least privilege; no implicit container/default interface |
| Service -> PostgreSQL/telemetry | Secret-store references, restricted DB roles, encrypted transport where deployed, redacted logs/metrics |

## Identity and authorization

Support POC local administration and scoped API keys behind an identity
provider abstraction that can later support OIDC/OAuth/Keycloak/SSO. Do not
couple domain policy to the POC identity mechanism. Define scopes for read,
configuration, route mutation, override-lock, GPIO output, audit, key/secret
administration, backup/restore, and worker administration. Enforce policy in
application services, not only UI or HTTP middleware.

The reference prompt supplies a static POC administrator credential. Treat it
as a bootstrap-only, insecure development default: never commit it as an
operational credential; require secret-based provisioning, force change or
disable default access before non-local use, and require TLS for network
authentication. Store password verifiers using a modern password-hashing
scheme. Store API-key hashes, show raw key once, scope/expire/revoke keys, and
never log them.

## Secrets, configuration, and audit

Persist secret references and masked configuration, not plaintext. Secret
read endpoints never return secret values. Redact authorization headers,
source credentials, API keys, and connection secrets from errors, audits,
traces, diagnostics, and backups. Sanitize Zephyr packet diagnostics and
Device Server payloads. Audit principal, action, resource, revision,
correlation/operation ID, timestamp, and safe diff; protect audit records from
ordinary mutation.

## Media/network controls

Require explicit interface/bind selection for every network source/destination;
validate address family, port collisions, multicast capability, and sender
identity. For Zephyr require exact configured IP/source-port and expected
SSRC/PT pairs. Bound packet/frame buffers, parsing work, queue sizes, and
diagnostic retention; reject malformed RTP/ADTS/GPIO without panics or
unbounded allocation. Never infer/trust a sender dynamically.

## Availability, privacy, and operations

Separate liveness/readiness, expose degradation without leaking configuration,
rotate/revoke credentials, back up with access controls, and document restore
and key-rotation behavior. Do not put personal/sensitive data in labels or
high-cardinality metrics. Define retention for audit, event replay,
idempotency, packet diagnostics, and telemetry before implementation.

## Security acceptance

Test anonymous/unauthorized access, scope boundaries, lock override policy,
revision conflicts, key expiration/revocation, secret non-disclosure and
redaction, rate/size limits, malformed packet bounds, sender spoof rejection,
worker generation fencing, plugin reconnect/auth/version errors, backup
protection, and password bootstrap/change behavior.
