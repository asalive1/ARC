# Changelog

## Unreleased

### Added
- Approved ARC architecture package and Phase 0 Rust/API/worker/media contract scaffolding.
- PostgreSQL initial migration, OpenAPI skeleton, Zephyr fixture area, and React health UI.
- Local Docker Compose stack for the API, UI, and PostgreSQL, plus a source dependency license inventory.
- Versioned WebSocket/JSON selected for API-to-worker control; credential provisioning remains open.

### Changed
- README and roadmap now describe ARC rather than the project template.
- API startup now requires PostgreSQL, applies SQLx migrations, and keeps its non-container bind address loopback-only.

### Fixed