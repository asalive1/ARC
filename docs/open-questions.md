# Open Questions Register

**Status:** Draft for decision. "Blocking" means resolve before the named gate,
not necessarily before all documentation can be reviewed.

| ID | Question / decision needed | Why it matters | Blocking gate |
|---|---|---|---|
| Q1 | Resolved 2026-11-22: Phase 1 uses the invoked prompt's ALSA/RTP inputs and RTP/SRT outputs, with an RTP input -> SRT output demo. Other technical details and Zephyr's later phase remain governed by V6; Zephyr live acceptance still requires Q3/Q4. | Records the user's explicit Phase 1 scope choice without changing unrelated V6 requirements. | Resolved for Phase 1 baseline |
| Q2 | Resolved: user approved the architecture package and Phase 0 implementation on 2026-11-22. | The invoked prompt's approval gate has been satisfied for Phase 0; later phases still require their normal gates. | Resolved for Phase 0 |
| Q3 | Provide/confirm audio and GPIO SSRC pairs for each of the five Zephyr channels. | V6 forbids assuming channel 1 identity applies to other channels. | **Blocking: live Zephyr acceptance** |
| Q4 | Provide actual Zephyr sender IP/source-port, interface/VLAN, port/channel inventory, and live test schedule. | Needed for exact sender validation and end-to-end acceptance. | **Blocking: live Zephyr acceptance** |
| Q5 | Which real network destination is the final Zephyr demo target: RTP (default), SRT MPEG-TS, or UDP MPEG-TS? | Required real output and test environment. | **Blocking: integrated acceptance** |
| Q6 | Resolved 2026-11-22: ALSA and RTP are Phase 1 inputs; RTP and SRT are Phase 1 outputs. The first end-to-end demo is RTP input -> SRT output. | Defines the initial adapter coverage and acceptance path. | Resolved for Phase 1 scope |
| Q7 | What is the approved first-run admin credential provisioning and recovery flow? Should a local default be entirely disabled? | Static default is unsafe; UI/API bootstrap must be testable. | **Blocking: non-local deployment** |
| Q8 | Resolved 2026-11-22: use versioned WebSocket/JSON; authenticate a worker service identity bound to its registered `workerId`. Credential provisioning/rotation remains Q13. | Freezes the transport and logical identity while keeping secret-provider choice separate. | Resolved for Phase 0 contract |
| Q9 | What native Device Server SDK/version, host process, plugin language/build ABI, vendor targets, and test environment are required? | Plugin deployment/build is independent and platform-specific. | **Blocking: native plugin acceptance** |
| Q10 | What are the authoritative semantics and validation values for Zephyr GPIO payload bytes 1-7 and group identity? | V6 requires profile validation but semantic names are unknown. | **Blocking: strict profile/live acceptance** |
| Q11 | What is the desired Device Server output failure policy for GPO actions, pulse cancellation, and reconnect reconciliation? | Safety and state divergence behavior. | **Blocking: GPIO integration acceptance** |
| Q12 | What latency/availability/retention targets apply to routing, event replay, audit, logs, metrics, and packet diagnostics? | Database sizing, replay contract, and operations. | **Blocking: production hardening** |
| Q13 | Which secret provider is required for local POC, Docker, Windows, and future AWS? | Configuration and deployable credential management. | **Blocking: deployable auth/secrets** |
| Q14 | Which backup destination, encryption, retention, restore authorization, and consistency guarantees are required? | Backup API and persistence recovery are in scope but details are incomplete. | **Blocking: backup/restore release** |
| Q15 | What licensing/feature model is expected in the POC, if any? | Licensing API appears in broad scope but feature semantics are absent. | **Blocking: licensing implementation** |
| Q16 | Resolved 2026-11-22: user approves LGPL-licensed GStreamer components and MPL-2.0 Haivision SRT for Phase 1, with GPL/AGPL components excluded. Exact package closure still requires per-target inventory. | Allows the selected pass-through path while retaining release-time license/SBOM review. | Resolved for Phase 1; image inventory remains a release gate |
| Q17 | Should route take/immediate mode, locks, permissions, undo and presets all be POC requirements, or which are production-ready interfaces only? | Supplemental routing scope is much larger than the initial vertical slice. | **Blocking: Phase 3 scope** |
| Q18 | What event replay retention and snapshot consistency guarantees are required? | WebSocket reconnect UX and durable event schema depend on it. | **Blocking: API contract freeze** |
| Q19 | What stable network-interface ID strategy should survive reboot, rename, Docker namespace, and OS differences? | Persisted interface selection must resolve correctly after restart. | **Blocking: network config contract** |
| Q20 | Resolved: PostgreSQL 16 and SQLx 0.8.6 are the scaffold baseline. | Reproducible schema/bootstrap and CI use the locked migration runner and PostgreSQL 16 validation. | Resolved for Phase 0 scaffold |
| Q21 | Resolved 2026-11-22: the first RTP -> SRT path uses RTP MPEG-TS (MP2T, payload type 33) -> SRT MPEG-TS pass-through; ALSA -> RTP uses 48 kHz stereo L16 PCM. | Defines the initial media formats without selecting an encoder. | Resolved for Phase 1 media profile |
| Q22 | Resolved 2026-11-22: add 48 kHz stereo RTP L24 input and output profiles alongside existing formats; only matching RTP L24 -> RTP L24 is a supported planned path, with endpoint-local dynamic payload types. | Adds L24 without implying bit-depth conversion or changing the RTP MP2T -> SRT demo. | Resolved for Phase 1 profile scope |

Resolve all questions marked blocking before their named gate; record decisions
with owner/date and update the [assumptions](assumptions.md),
[risk register](risk-register.md), and traceability matrix.
