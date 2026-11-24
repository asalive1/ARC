# ARC - Audio Routing Controller

ARC is an API-first audio routing and signal-control platform. The approved
architecture and runnable Phase 0 scaffold are in place. The initial Phase 1
slice persists source/destination drafts, plans compatible media profiles, and
validates draft configurations. The operator UI can now reserve a worker for a
validated single route, Start or Stop it, or cancel it before it starts. An
opt-in worker profile provides the worker command client and GStreamer adapter.
Assignment Start/Stop now coordinates source and destination enabled state with
worker acknowledgements. Multi-route editing and end-to-end deployed media
transport remain outstanding.

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



