# Risk Register

**Status:** Draft for approval
Likelihood/impact are preliminary qualitative ratings, not measured values.

| ID | Risk | L/I | Mitigation / evidence required | Owner / gate |
|---|---|---|---|---|
| R1 | V6 and invoked master disagree on Phase 1 inputs/outputs and Zephyr timing | M/H | Approve one acceptance sequence; preserve V6 precedence and record deviation before phase plan baseline | Product / before Phase 1 |
| R2 | Zephyr channel-specific SSRCs and field semantics are incomplete | H/H | Configure per-channel profile, validate neutral fields, synthetic fixtures, live sender evidence; no guessed semantics | Media / before Zephyr acceptance |
| R3 | Selected network interface behavior differs under Docker, Windows, VLAN, bond, or multicast | H/H | Stable interface IDs, host inventory, explicit binds, target-host probes, document platform constraints | Platform / Phase 0-1 |
| R4 | GStreamer plugin/codec availability and licenses differ by OS/architecture | M/H | Capability discovery, pinned build images, license/SBOM review, real media test on target architectures | Media / before image release |
| R5 | Route apply spans DB and external pipelines, so cannot be truly atomic | H/H | Operation/saga model, prepare/observe/compensate, generations, per-item results, failure injection | Architecture / Phase 1 |
| R6 | Slow output blocks source or sibling routes | M/H | Per-destination bounded queues and isolation; pressure/failure tests | Media / Phase 1 |
| R7 | Worker lease loss or stale generation can create duplicate/stale pipelines | M/H | Fencing tokens, heartbeat/lease policy, reconcile and drain tests | Control / Phase 1 |
| R8 | GPIO timing varies with decoder, network jitter, clocks, and output device | H/H | Explicit timing modes/offsets, measure actual dispatch, configurable targets, live acceptance | Signals / Phase 2-5 |
| R9 | Default POC admin credential is unsafe outside local development | H/H | Secret-based bootstrap, forced rotation/disable, TLS, negative auth tests | Security / before deployment |
| R10 | POC scope spans media, routing, GPIO, plugin, UI, ARM64 and Windows | H/H | Vertical slices, capability-based exclusions, phase exit criteria, no speculative adapters | Product / continuous |
| R11 | 64x64 matrix and event fan-out cause UI/API load or stale view | M/M | Virtualized grid, cursor/replay snapshot, bounded subscriptions, concurrent-user tests | UI / Phase 3 |
| R12 | Schema/recovery model omits retention, event replay, auth, licensing, or backup metadata | M/H | Review ERD/questions; migration/restore tests; PostgreSQL-only operational design | Data / Phase 0 |
| R13 | Native Device Server SDK/build ABI is host/vendor-specific | M/H | Separate plugin artifact and build path, protocol contract tests, vendor acceptance environment | Integration / Phase 5 |
| R14 | Open-source policy conflicts with AAC codec or plugin dependencies | M/H | Dependency/license review for every target image, alternative decoder path, signed-off SBOM | Legal/Media / before release |
| R15 | No Git remote is configured; phase release workflow cannot be published until the approved remote is confirmed | Certain/H | Confirm approved repository/remote before release work; do not claim commits/tags/pushes that have not occurred | Owner / before Phase 0 exit |
| R16 | Backup/restore, audit and diagnostic retention introduce sensitive data exposure | M/H | Redaction, encryption/access control, retention and restore tests | Security/Ops / Phase 6 |
