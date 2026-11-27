# ARC - Audio Routing Controller

ARC is an API-first audio routing and signal-control platform. The approved
architecture and runnable Phase 0 scaffold are in place. The initial Phase 1
slice persists source/destination drafts, plans compatible media profiles, and
validates draft configurations. The operator UI can now reserve a worker for a
validated single route, Start or Stop it, or cancel it before it starts. An
opt-in worker profile provides the worker command client and GStreamer adapter.
Assignment Start/Stop now coordinates source and destination enabled state with
worker acknowledgements. Phase 2 adds crosspoint routing, shared-source fan-out,
durable multi-route execution, and saved salvos. Deployed Linux ALSA-loopback
to RTP stereo fan-out and Unroute are verified; external-endpoint and full
deployment acceptance remain outstanding.

## Crosspoint routing and salvos

- Left-click a crosspoint to highlight it without changing live media.
  Right-click it (or focus it and press Shift+F10) for **Take** and **Unroute**.
  Take starts the selected input/output route; Unroute disconnects that output.
  Replacing another input still requires explicit confirmation.
- **Salvo**, below GPIO Configuration in the sidebar, has its saved-salvo
  dropdown, **Take**, and **Save** controls above the matrix. It starts with
  an empty selection and **Select Salvo** in the dropdown; live routes are
  not overlaid. Each return to this screen resets the dropdown and draft.
  Click a crosspoint to cycle through **blue Take**, **red Unroute**, and clear.
  Red disconnects that output regardless of its connected input; clear leaves
  the output unchanged. Select one action per output, then **Save** and name
  the salvo. Choosing a saved salvo displays its saved blue/red actions.
- Save stores only the selected actions, not every live route. It does not
  change live media. Save as new, update, reload, and delete are available.
  Saved salvos use PostgreSQL and the `/api/v1/presets` API.
  Up to 100 salvos are supported, each with 1-64 distinct output assignments
  and a unique, trimmed name of 1-100 characters. Force mode is not implemented.
- Take executes the saved salvo, not unsaved edits. Loading a salvo captures
  the displayed destination revisions; strict execution rejects stale route
  or salvo revisions. Reload and review before retrying a conflict. Unlisted
  destinations remain unchanged. Force execution is not available.
- Desired changes are recorded atomically, but media execution is independent
  per destination, not an all-or-nothing switch. Review operation and
  destination outcomes rather than assuming a request means media is running.
  Replacements prepare the new path before the hard cut and attempt to restore
  the previous route after a failed Start.
- A destination with started execution, cleanup, or restoration still pending
  rejects new Take, Salvo, direct-write, and Unroute requests with HTTP 409
  `destination_busy`. Retry the accepted operation with its original
  idempotency key rather than submitting a conflicting route. Cleanup retains
  the replacement's capacity until its Stop is acknowledged.
- Unroute is revision-guarded and durable. Busy destinations can reject a
  conflicting command with HTTP 409. Stop acknowledgement, not command
  acceptance, releases capacity and confirms stopped media. If no assignment
  can confirm Stop, desired state is removed but the operation fails with
  `stop_unconfirmed`; review that outcome rather than assuming silence.
- The old bottom Take controls are removed. Compatibility checks and manual
  worker lifecycle/capacity controls remain under **Advanced routing diagnostics**.

## Input and output configuration

- **New input/output** is a collapsed, full-width panel. Expand its arrow to
  create a disabled draft.
- Existing resources fill the available content width. Expand a row to edit
  it inline, save a new draft revision, validate the saved revision, or delete
  the resource. Cancel discards unsaved edits.
- Enabled resources can be expanded for inspection, but their fields, Save
  and Delete controls are disabled with an explanation. Unroute all assignments
  using the resource before editing; opening an editor never stops media.
- Delete requires confirmation and the displayed configuration revision.
  Only disabled, unused resources can be permanently deleted. Routes,
  **including removed route history**, assignments, signal mappings and saved
  salvos block deletion. History is never cascaded away, and deletion cannot
  be undone. Editing preserves prior configuration revisions.

See [test-server readiness](docs/deployment.md#isolated-test-server-readiness)
for the steps needed before an isolated deployment.
The [media-enabled test deployment](docs/deployment.md#linux-dual-network-media-worker)
now runs a non-root dual-network worker with ALSA capture and trusted WSS.

## Architecture review

Start with [the system architecture](docs/system-architecture.md), then review
the [open questions](docs/open-questions.md) and
[delivery plan](docs/delivery-plan.md). The complete pre-code document set is
listed in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). See the
[technology licensing guide](docs/licensing.md) for current and planned
third-party technologies.

## Development

Use VS Code Tasks:

- Project Status
- Save Checkpoint
- Build / Deploy
- Backup To GitHub
- Create Release

For the runnable local development stack, follow the
[Compose deployment guide](docs/deployment.md).

## Phase 0 validation

The local toolchains may be run directly or through Docker:

```powershell
cargo fmt --all -- --check
cargo test --locked --workspace
Set-Location ui
npm ci
npm test
npm run build
npm audit --audit-level=moderate
```

PostgreSQL is the only supported database. The initial schema is in
`database/migrations`; its execution path is implemented by the SQLx
infrastructure crate and applied by the API at startup. Running the API
directly requires `DATABASE_URL`; Compose configures it for the local database.

## Releases

See [CHANGELOG.md](CHANGELOG.md).




