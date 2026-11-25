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
| Argon2 | 0.5.3 | MIT OR Apache-2.0 | Administrator password hashing |
| Axum | 0.8.9 | MIT | HTTP API |
| base64 | 0.22.1 | MIT OR Apache-2.0 | Opaque administrator session tokens |
| Chrono | 0.4.45 | MIT OR Apache-2.0 | Dates and times |
| rand | 0.8.8 | MIT OR Apache-2.0 | Cryptographically secure authentication tokens and password salts |
| rustls | 0.23.46 | MIT OR Apache-2.0 | Worker WebSocket TLS client |
| rustls-pemfile | 2.2.0 | MIT OR Apache-2.0 | Parse optional worker CA trust bundles |
| Serde / serde_json | 1.0.229 / 1.0.154 | MIT OR Apache-2.0 | Serialization |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | One-way hashes for bootstrap, recovery and session credentials |
| SQLx | 0.8.6 | MIT OR Apache-2.0 | PostgreSQL access and migrations |
| sysinfo | 0.37.2 | MIT | Worker CPU and memory utilization telemetry |
| subtle | 2.6.1 | BSD-3-Clause | Constant-time worker and administrator token comparisons |
| Tokio | 1.54.1 | MIT | Asynchronous runtime |
| Tower | 0.5.3 | MIT | HTTP service testing |
| tracing / tracing-subscriber | 0.1.44 / 0.3.23 | MIT | Logging and tracing |
| UUID | 1.29.3 | Apache-2.0 OR MIT | Identifiers |
| webpki-roots | 1.0.12 | MPL-2.0 | Bundled public TLS trust roots |
| Emotion React / Styled | 11.14.0 / 11.14.1 | MIT | UI styling |
| futures-util | 0.3.34 | MIT OR Apache-2.0 | Worker WebSocket client message handling and API session tests |
| GStreamer Rust bindings | 0.25.4 | MIT OR Apache-2.0 | Worker media-plane adapter |
| gstreamer-app Rust bindings | 0.25.2 | MIT OR Apache-2.0 | API-dispatched live RTP/SRT integration-test receivers |
| Material UI | 7.3.11 | MIT | UI components |
| TanStack React Query | 5.108.3 | MIT | UI data fetching |
| React / React DOM | 19.3.1 / 19.3.1 | MIT | UI runtime |
| React type definitions / React DOM type definitions | 19.3.0 / 19.3.0 | MIT | UI development |
| Vite React plugin | 5.2.0 | MIT | UI build |
| TypeScript | 5.9.3 | Apache-2.0 | UI type checking |
| Vite | 7.3.8 | MIT | UI build and development server |
| Vitest | 5.1.1 | MIT | UI tests |
| tokio-tungstenite | 0.29.0 | MIT | Worker WebSocket client and API session tests |
| url | 2.5.9 | MIT OR Apache-2.0 | SRT URI construction and host validation |

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
Persisting route revision references and configuration snapshots adds only a
PostgreSQL migration and no external dependencies.
Exposing revision-pinned assignment planning uses the existing Chrono, Serde,
Axum and infrastructure dependencies; Chrono is now a direct dependency of
the API crate. No third-party package or new technology was added.
Worker command identity and generation-fencing groundwork use the existing
Serde and async-trait dependencies; the PostgreSQL fence and command ledger use
existing SQLx support. No third-party package was added.
The API command dispatcher reuses the workspace `arc-worker` crate and existing
Tokio/WebSocket dependencies; it adds no third-party package.
Provider-backed worker credential verification adds `subtle` for constant-time
comparison. It was already present transitively in the lockfile; its direct
use is now listed above.
The local JSON environment credential provider reuses Serde and adds no
third-party packages.
Administrator authentication adds Argon2 password hashing, cryptographic token
generation, and token hashing. Cookie sessions use HttpOnly/SameSite settings
and CSRF tokens; no additional authentication framework was introduced.
The API WebSocket feature uses Axum's existing WebSocket support. The
tokio-tungstenite and futures-util direct dependencies support the worker's
production WebSocket client and API session tests. The GStreamer Rust bindings
and `url` support the isolated `arc-gstreamer` adapter. Their locked packages,
together with the other transitive WebSocket and GStreamer packages, remain
subject to the release-time dependency inventory.
Optional worker private-CA trust uses `rustls` and `rustls-pemfile`, while
`webpki-roots` preserves the bundled public trust roots.
The optional Certbot sidecar adds an ACME issuance/renewal image but no Rust
workspace package; record its exact image digest and package SBOM for releases.

Key WebSocket transitive packages include `tungstenite` 0.29.0
(MIT OR Apache-2.0) and `data-encoding` 2.11.1 (MIT), based on the locked
package metadata.

## Deployment and build technologies

| Technology | Current use | License information |
|---|---|---|
| PostgreSQL 16 | Database in Compose | [PostgreSQL License](https://www.postgresql.org/about/licence/); preserve the required notices. |
| NGINX 1.29 | Serves the UI in the runtime image | [BSD-2-Clause](https://nginx.org/LICENSE); preserve the applicable source or binary notices. |
| Certbot 5.10.0 | Optional ACME certificate issuance and renewal image | [Apache-2.0](https://github.com/certbot/certbot/blob/master/LICENSE.txt); inventory the exact image and bundled Alpine packages for release. |
| GStreamer 1.22 runtime | Worker RTP, ALSA and SRT pipeline plugins | Required components and selected source files are LGPL-2+. The core parser is generated with Bison's exception and ships through `libgstreamer1.0-0`. SRT is MPL-2.0 as approved for Phase 1. Preserve notices and source obligations. |
| Rust toolchain | API build image (`rust:1-slim`) and worker build image (`rust:1-bookworm`) | Rust is generally [MIT OR Apache-2.0](https://github.com/rust-lang/rust/blob/master/LICENSE-MIT); toolchain components and bundled dependencies may differ. |
| curl/libcurl | API runtime healthcheck package | curl license (MIT-style); preserve applicable notices. |
| Node.js 22 | UI build image | Node.js is generally [MIT](https://github.com/nodejs/node/blob/main/LICENSE); bundled components have their own notices. |
| Debian Bookworm / Alpine Linux | Base images for API, worker, UI, database and build stages | Distributions contain many separately licensed packages. Inventory the actual packages in each exact image; neither image has a single blanket license. |

Image tags are not a complete license inventory: base images and installed OS
packages can change, and build-stage dependencies differ from distributed
runtime contents. Record image digests, package versions, licenses, and
notices per target artifact.

## Phase 1 media runtime

The opt-in worker image installs GStreamer core, the Base support library
(`libgstreamer-plugins-base1.0-0`) and the ALSA plugin, but not the full Base,
Good or Bad plugin bundles. It copies only `libgstapp.so`,
`libgstaudioconvert.so`, `libgstudp.so`, `libgstrtp.so`,
`libgstrtpmanager.so` and `libgstsrt.so` from non-shipped Debian plugin
extraction stages. The SRT runtime library is installed separately.
The user approved LGPL GStreamer components and
[Haivision SRT under MPL-2.0](https://github.com/Haivision/srt/blob/master/LICENSE)
for Phase 1, with GPL/AGPL media components excluded from the media stack and
its dependencies. This exclusion does not apply to base OS packages; their
licenses, notices and obligations still require inventory and review.
GStreamer's
[licensing guidance](https://github.com/GStreamer/gstreamer/blob/main/subprojects/gst-docs/markdown/frequently-asked-questions/licensing.md)
describes the framework and plugin licensing; individual plugins and external
libraries must still be checked. LGPL and MPL distribution obligations,
including applicable notices and source/modification requirements, need
review for the actual artifacts and linking/distribution method. Codec and
patent considerations are separate from software copyright licenses.

The Debian `gstreamer1.0-plugins-bad` package also contains unused GPL plugins
(`dtsdec`, `faad`, `mpeg2enc`, `mplex`, `resindvd`, and `x265`) and plugins with
unknown license metadata (`mpegpsdemux`, `rtponvif`, and `voamrwbenc`). These
are excluded from the final worker image. The required app, audioconvert,
coreelements, ALSA, UDP, RTP, RTP-manager and SRT plugins report LGPL metadata
on AMD64 and emulated ARM64; the separately licensed MPL-2.0 SRT runtime
library is installed. The final images also omit `libaa1`, `libcaca0` and
`libdv4` from the full good-plugin bundle; `libdvdnav4` and `libdvdread8` are
absent as well. Full per-target OS dependency/license and notice review
remains a release gate. The targeted SBOM and plugin findings are summarized in
[dependency-license-inventory](dependency-license-inventory.md). Current Linux
image SBOMs describe local builds only: no release image digests or Windows
worker artifact are available for release review.

Debian's GStreamer core copyright metadata labels the generated parser header
`gst/parse/grammar.tab.h` GPL-3+. Debian's Meson build generates
`grammar.tab.c` and `grammar.tab.h` with Bison, includes the header from
`parse.l`, and adds both parser outputs to the `gstreamer-1.0` library. The
Debian `libgstreamer1.0-0` package ships that shared library, not the generated
header separately. Reproduction with Bookworm Bison 3.8.2 confirmed the Bison
special-exception notice in both generated outputs; Bison's published
[conditions](https://www.gnu.org/software/bison/manual/html_node/Conditions.html)
confirm the exception applies to output containing that notice. The technical
generator and package-to-binary scope is resolved; package notices and full
release license closure still require per-artifact review.

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
