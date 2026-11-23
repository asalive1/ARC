# Assumptions Register

**Status:** Approved baseline. Items marked provisional require confirmation
or validation; none should be silently converted into a product guarantee.

| ID | Assumption | Basis / status | Consequence if false |
|---|---|---|---|
| A1 | Master Prompt V6 is the technical source of truth where earlier master prompts conflict | V6 precedence section; governing draft assumption | Rebaseline phases and requirements |
| A2 | Approval is required before implementation | Explicit instruction in the invoked master prompt; honored for this task | Implementation may proceed only after an explicit approval decision |
| A3 | PostgreSQL is the sole supported durable database | V6 and invoked prompt; confirmed | No SQLite or database switching abstractions |
| A4 | API and worker are separate logical/process services even when colocated | V6; confirmed architecture constraint | Changes deployment and failure boundaries |
| A5 | POC begins with one local worker but uses scheduler, assignment, lease, capacity, and generation contracts | V6; confirmed | Multiworker evolution would require rework |
| A6 | GStreamer is the primary media framework behind framework-neutral ports | V6; confirmed | Alternative media runtime requires adapter/contract approval |
| A7 | Worker owns network-interface discovery and media socket binding | Network supplemental + V6; confirmed | Host/container topology changes |
| A8 | Linux Docker may require worker host networking; API/UI/PostgreSQL need not share that mode | Network supplemental; target-specific | Document deployment and firewall changes per host |
| A9 | Network sources/destinations explicitly select stable interface ID or valid bind address | Network supplemental/V6; confirmed | Config schema and operational UX change |
| A10 | Zephyr POC supports AAC-LC/ADTS, 48 kHz stereo, five concurrent channels, configured identity and exact sender pair | V6-approved profile | Provider configuration and tests need revision |
| A11 | Sample Zephyr SSRCs are not universal; per-channel pair is configured | V6; confirmed | Incorrect channel assumptions reject valid traffic |
| A12 | GPIO active-low mapping and supported pin bits 0-4 are adapter profile defaults, not global GPIO semantics | V6; confirmed | Device-specific polarity/pin model changes |
| A13 | Synthetic fixtures/simulator substitute for absent PCAP during automated tests; live sender still required for final acceptance | V6; confirmed | Acceptance requires additional capture/environment |
| A14 | WebSocket is read-only for client events; REST remains mutation channel | API supplemental/V6; confirmed | UI/control contracts must be redesigned if command WSS is required |
| A15 | Device Server plugin is a separately built/deployed native artifact using versioned authenticated WebSocket JSON | V6 and invoked prompt; preferred POC protocol | Vendor SDK/host integration may need adaptation |
| A16 | Docker is the POC deployment baseline; ARM64/AMD64 and Windows x64 targets are validated per actual dependencies | V6; confirmed target, dependencies conditional | Platform support statements narrowed |
| A17 | V6's phase order is the proposed roadmap; older Phase 1 ordering is a pending conflict | V6 supersedes prior prompts, but invoked prompt differs | Acceptance/demo order changes |
| A18 | No AWS resource provisioning occurs in POC; AWS-facing behavior remains behind infrastructure-neutral ports | V6; confirmed | Deployment scope expands materially |
| A19 | OpenAPI-generated TypeScript SDK and C# generation compatibility are desired; generator choice is open | V6/API supplemental | Tooling and compatibility policy need decision |
| A20 | Licensing, backup storage/retention, audit/event retention, and availability targets remain product decisions | Not fully specified in reviewed sources | Data model, operations, and deployment design may change |
