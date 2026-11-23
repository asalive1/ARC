# Open Questions Register

**Status:** Draft for decision. "Blocking" means resolve before the named gate,
not necessarily before all documentation can be reviewed.

| ID | Question / decision needed | Why it matters | Blocking gate |
|---|---|---|---|
| Q1 | Approve V6's Phase 1 generic RTP->RTP/null slice, or retain the invoked prompt's ALSA/RTP->RTP/SRT demo? Confirm when Zephyr enters the acceptance sequence. | Changes scope, adapter order, and the first end-to-end demonstration. | **Blocking: implementation/Phase 1 baseline** |
| Q2 | Does the explicit approval-before-code gate in the invoked prompt control over V6's "continue implementation" wording after document generation? | Conflicting execution instructions. This package conservatively stops at the approval gate. | **Blocking: implementation authorization** |
| Q3 | Provide/confirm audio and GPIO SSRC pairs for each of the five Zephyr channels. | V6 forbids assuming channel 1 identity applies to other channels. | **Blocking: live Zephyr acceptance** |
| Q4 | Provide actual Zephyr sender IP/source-port, interface/VLAN, port/channel inventory, and live test schedule. | Needed for exact sender validation and end-to-end acceptance. | **Blocking: live Zephyr acceptance** |
| Q5 | Which real network destination is the final Zephyr demo target: RTP (default), SRT MPEG-TS, or UDP MPEG-TS? | Required real output and test environment. | **Blocking: integrated acceptance** |
| Q6 | Which Phase 1 input/output types are mandatory for the POC release versus planned capability? | Priorities and early phase scope differ among docs. | **Blocking: release scope** |
| Q7 | What is the approved first-run admin credential provisioning and recovery flow? Should a local default be entirely disabled? | Static default is unsafe; UI/API bootstrap must be testable. | **Blocking: non-local deployment** |
| Q8 | Which worker control transport and service identity should be used between API and worker (local IPC, gRPC, or versioned WebSocket/JSON)? | Container and multi-worker contract. | **Blocking: Phase 0 contract freeze** |
| Q9 | What native Device Server SDK/version, host process, plugin language/build ABI, vendor targets, and test environment are required? | Plugin deployment/build is independent and platform-specific. | **Blocking: native plugin acceptance** |
| Q10 | What are the authoritative semantics and validation values for Zephyr GPIO payload bytes 1-7 and group identity? | V6 requires profile validation but semantic names are unknown. | **Blocking: strict profile/live acceptance** |
| Q11 | What is the desired Device Server output failure policy for GPO actions, pulse cancellation, and reconnect reconciliation? | Safety and state divergence behavior. | **Blocking: GPIO integration acceptance** |
| Q12 | What latency/availability/retention targets apply to routing, event replay, audit, logs, metrics, and packet diagnostics? | Database sizing, replay contract, and operations. | **Blocking: production hardening** |
| Q13 | Which secret provider is required for local POC, Docker, Windows, and future AWS? | Configuration and deployable credential management. | **Blocking: deployable auth/secrets** |
| Q14 | Which backup destination, encryption, retention, restore authorization, and consistency guarantees are required? | Backup API and persistence recovery are in scope but details are incomplete. | **Blocking: backup/restore release** |
| Q15 | What licensing/feature model is expected in the POC, if any? | Licensing API appears in broad scope but feature semantics are absent. | **Blocking: licensing implementation** |
| Q16 | What decoder/encoder and GStreamer plugin packages are approved under the permissive-license requirement on each platform? | AAC and other elements may have different availability/licenses. | **Blocking: target images/release** |
| Q17 | Should route take/immediate mode, locks, permissions, undo and presets all be POC requirements, or which are production-ready interfaces only? | Supplemental routing scope is much larger than the initial vertical slice. | **Blocking: Phase 3 scope** |
| Q18 | What event replay retention and snapshot consistency guarantees are required? | WebSocket reconnect UX and durable event schema depend on it. | **Blocking: API contract freeze** |
| Q19 | What stable network-interface ID strategy should survive reboot, rename, Docker namespace, and OS differences? | Persisted interface selection must resolve correctly after restart. | **Blocking: network config contract** |
| Q20 | Is a database migration tool/SQLx version or PostgreSQL major version already selected? | Reproducible schema/bootstrap and CI. | **Blocking: Phase 0 implementation** |

Resolve all questions marked blocking before their named gate; record decisions
with owner/date and update the [assumptions](assumptions.md),
[risk register](risk-register.md), and traceability matrix.
