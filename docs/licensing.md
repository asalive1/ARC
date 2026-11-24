# Technology Licensing

**Status:** Engineering inventory, not legal advice or release approval.
**Snapshot:** 2026-11-24, based on the checked-in manifests and lockfiles.

This guide covers the technologies currently used by ARC and the media
technologies approved for future work. The more detailed
[dependency license inventory](dependency-license-inventory.md) records an
earlier full-graph metadata review; regenerate it for each release rather
than treating its package counts as current.

Update this guide whenever project technologies, dependencies, licenses, or
deployment images change. Record what was added, removed, or changed, and
distinguish planned components from those actually shipped. Refresh the
full-graph inventory and release review when the dependency closure changes.

## ARC's own code

The Rust workspace declares `MIT OR Apache-2.0` in `Cargo.toml`. This is a
package metadata declaration, not a third-party license or evidence of
release approval. The repository currently has no root license text; the
project owner should confirm the intended product license and include the
appropriate license and copyright notices before distributing ARC. The UI
package is marked private and does not declare a separate license.

## Current direct dependencies

Versions below are resolved versions in `Cargo.lock` and
`ui/package-lock.json`; licenses are package metadata, not independent legal
verification. An `OR` expression offers alternative licenses, whereas `AND`
requires attention to all listed terms.

| Technology | Locked version | Declared license | Use |
|---|---:|---|---|
| async-trait | 0.1.92 | MIT OR Apache-2.0 | Rust async interfaces |
| Axum | 0.8.9 | MIT | HTTP API |
| Chrono | 0.4.45 | MIT OR Apache-2.0 | Dates and times |
| Serde / serde_json | 1.0.229 / 1.0.154 | MIT OR Apache-2.0 | Serialization |
| SQLx | 0.8.6 | MIT OR Apache-2.0 | PostgreSQL access and migrations |
| Tokio | 1.54.1 | MIT | Asynchronous runtime |
| Tower | 0.5.3 | MIT | HTTP service testing |
| tracing / tracing-subscriber | 0.1.44 / 0.3.23 | MIT | Logging and tracing |
| UUID | 1.29.3 | Apache-2.0 OR MIT | Identifiers |
| Emotion React / Styled | 11.14.0 / 11.14.1 | MIT | UI styling |
| Material UI | 7.3.11 | MIT | UI components |
| TanStack React Query | 5.108.3 | MIT | UI data fetching |
| React / React DOM | 19.3.1 / 19.3.1 | MIT | UI runtime |
| React type definitions / React DOM type definitions | 19.3.0 / 19.3.0 | MIT | UI development |
| Vite React plugin | 5.2.0 | MIT | UI build |
| TypeScript | 5.9.3 | Apache-2.0 | UI type checking |
| Vite | 7.3.8 | MIT | UI build and development server |
| Vitest | 5.1.1 | MIT | UI tests |

These are only direct dependencies. Transitive packages, build tooling,
generated assets and target-specific dependencies can have different terms.
For example, the earlier inventory identifies CC-BY-4.0 compatibility data
and Unicode and CDLA-licensed Rust dependencies.

The PostgreSQL worker registry uses the already-listed async-trait, Chrono,
Serde and SQLx packages. The capacity reservation adapter and its PostgreSQL
migration, and the planned assignment adapter and its migration, also use
existing dependencies; none adds third-party packages.
The worker drain-request and PostgreSQL scheduler foundations also add no
dependencies or technologies. The Phase 1 requirement estimator likewise uses
existing project code and adds no third-party packages.
Exposing the estimate in the route planner and UI also adds no dependencies.
The atomic pipeline-planning workflow uses the existing SQLx and domain code
and adds no third-party packages.

## Deployment and build technologies

| Technology | Current use | License information |
|---|---|---|
| PostgreSQL 16 | Database in Compose | [PostgreSQL License](https://www.postgresql.org/about/licence/); preserve the required notices. |
| NGINX 1.29 | Serves the UI in the runtime image | [BSD-2-Clause](https://nginx.org/LICENSE); preserve the applicable source or binary notices. |
| Rust toolchain | API build image (`rust:1-slim`) | Rust is generally [MIT OR Apache-2.0](https://github.com/rust-lang/rust/blob/master/LICENSE-MIT); toolchain components and bundled dependencies may differ. |
| Node.js 22 | UI build image | Node.js is generally [MIT](https://github.com/nodejs/node/blob/main/LICENSE); bundled components have their own notices. |
| Debian Bookworm / Alpine Linux | Base images for API, UI, database and build stages | Distributions contain many separately licensed packages. Inventory the actual packages in each exact image; neither image has a single blanket license. |

Image tags are not a complete license inventory: base images and installed OS
packages can change, and build-stage dependencies differ from distributed
runtime contents. Record image digests, package versions, licenses, and
notices per target artifact.

## Approved for future Phase 1 media work

GStreamer and SRT are **not yet included in the current build or runtime**.
The user approved LGPL GStreamer components and
[Haivision SRT under MPL-2.0](https://github.com/Haivision/srt/blob/master/LICENSE)
for Phase 1, with GPL/AGPL components excluded. GStreamer's
[licensing guidance](https://github.com/GStreamer/gstreamer/blob/main/subprojects/gst-docs/markdown/frequently-asked-questions/licensing.md)
describes the framework and plugin licensing; individual plugins and external
libraries must still be checked. LGPL and MPL distribution obligations,
including applicable notices and source/modification requirements, need
review for the actual artifacts and linking/distribution method. Codec and
patent considerations are separate from software copyright licenses.

RTP, SRT, MPEG-TS, ALSA and audio format names in ARC profiles do not by
themselves establish the license of a particular implementation. Inventory
the actual libraries, plugins, firmware and OS packages when selected.
The Device Server SDK and other future integrations are not inventoried or
approved here.

## Release checklist

1. Confirm ARC's own license and include its license texts and notices.
2. Generate an SBOM and complete license/notice inventory from the locked
   dependencies and exact build and runtime artifacts for each supported target.
3. Review all transitive dependencies, bundled assets, image packages and
   license expressions against the project's GPL/AGPL exclusion and any other
   applicable policy; resolve unknown licenses before release.
4. Include required third-party notices, license texts, attribution and any
   applicable source-offer or modification information with distribution.
5. Obtain appropriate legal and owner approval; this guide is not a substitute
   for that review.
