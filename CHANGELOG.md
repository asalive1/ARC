# Changelog

## Unreleased

### Added
- Approved ARC architecture package and Phase 0 Rust/API/worker/media contract scaffolding.
- PostgreSQL initial migration, OpenAPI skeleton, Zephyr fixture area, and React health UI.
- Local Docker Compose stack for the API, UI, and PostgreSQL, plus a source dependency license inventory.
- Versioned WebSocket/JSON selected for API-to-worker control; credential provisioning remains open.
- Phase 1 media profile selected: RTP MP2T to SRT MPEG-TS pass-through; LGPL GStreamer and MPL-2.0 SRT approved, with GPL/AGPL excluded.
- Added RTP L24 input/output profiles and matching RTP L24 route planning without L16/L24 conversion.

### Changed
- README and roadmap now describe ARC rather than the project template.
- API startup now requires PostgreSQL, applies SQLx migrations, and keeps its non-container bind address loopback-only.

### Fixed