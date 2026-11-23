# ARC - Audio Routing Controller

ARC is an API-first audio routing and signal-control platform. The architecture
package is approved, and Phase 0 scaffolding is in progress. The API liveness
shell and UI health view run; media pipelines, persistence-backed services, and
operator workflows are not implemented yet.

## Architecture review

Start with [the system architecture](docs/system-architecture.md), then review
the [open questions](docs/open-questions.md) and
[delivery plan](docs/delivery-plan.md). The complete pre-code document set is
listed in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Development

Use VS Code Tasks:

- Project Status
- Save Checkpoint
- Build / Deploy
- Backup To GitHub
- Create Release

## Phase 0 validation

The local toolchains may be run directly or through Docker:

```powershell
cargo run -p arc-api
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
infrastructure crate.

## Releases

See [CHANGELOG.md](CHANGELOG.md).



