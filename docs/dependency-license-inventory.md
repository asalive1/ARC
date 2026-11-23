# Dependency License Inventory

**Status:** Source dependency metadata reviewed; not a release approval.
**Inventory date:** 2026-11-22

## Scope and method

This snapshot covers the locked Rust workspace dependency graph and the
installed UI dependency graph from `ui/package-lock.json`. Rust metadata came
from `cargo metadata --locked --format-version 1`; npm packages were installed
with `npm ci` and read from package metadata. Target-specific Rust packages in
the lockfile are included. No package in either graph had a missing license
field.

The inventory does not include container base-image operating-system
packages, GStreamer/codec plugins, native audio backends, or the Device Server
SDK. Those components are not selected yet and must be inventoried for each
target image before release. This metadata review is not legal advice or a
signed-off SBOM.

## Rust workspace

The locked graph contains 234 registry packages. License expressions reported
by their package metadata:

| SPDX expression | Packages |
|---|---:|
| MIT OR Apache-2.0 | 131 |
| MIT | 35 |
| Apache-2.0 OR MIT | 20 |
| Unicode-3.0 | 18 |
| MIT/Apache-2.0 | 6 |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 3 |
| BSD-2-Clause OR Apache-2.0 OR MIT | 2 |
| ISC | 2 |
| CDLA-Permissive-2.0 | 2 |
| Apache-2.0 OR BSL-1.0 OR MIT | 2 |
| Unlicense OR MIT | 2 |
| Zlib | 1 |
| Zlib OR Apache-2.0 OR MIT | 1 |
| Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 1 |
| Apache-2.0 AND ISC | 1 |
| Apache-2.0/MIT | 1 |
| MIT AND BSD-3-Clause | 1 |
| Apache-2.0 OR ISC OR MIT | 1 |
| MIT OR Apache-2.0 OR LGPL-2.1-or-later | 1 |
| Apache-2.0 | 1 |
| BSD-3-Clause | 1 |
| Apache-2.0 OR BSL-1.0 | 1 |

The LGPL expression is an alternative license choice alongside MIT and Apache
2.0. No GPL-only or AGPL-only license expression was found.

## UI dependency graph

The installed graph contains 151 unique packages:

| License | Packages |
|---|---:|
| MIT | 135 |
| ISC | 8 |
| BSD-3-Clause | 4 |
| Apache-2.0 | 3 |
| CC-BY-4.0 | 1 |

The CC-BY-4.0 package is `caniuse-lite` 1.0.30001827, compatibility data used
by the frontend build toolchain. Preserve its attribution if redistributing
that data or a build artifact containing it. The lockfile inventory does not
by itself establish whether a particular build output includes the data.

Unicode-3.0 and CDLA-Permissive-2.0 are also present in the Rust graph. Keep
third-party notices with any redistributions that include their covered
materials.

## Phase 0 follow-up

Before a target image or release is approved, regenerate this review from the
locked manifests and include image SBOM/license data for the exact Linux
AMD64, Linux ARM64, and Windows x86-64 artifacts. Resolve the codec/plugin
license question in [open questions](open-questions.md) before selecting media
dependencies.

The user approved LGPL GStreamer components and the official
[Haivision SRT license](https://github.com/Haivision/srt/blob/master/LICENSE)
is MPL-2.0 for Phase 1, excluding GPL/AGPL components. GStreamer describes its framework and newly written plugins as
LGPL in its
[licensing FAQ](https://github.com/GStreamer/gstreamer/blob/main/subprojects/gst-docs/markdown/frequently-asked-questions/licensing.md).
The exact plugin and OS package closure still needs per-target inventory before
release. These dependencies have not yet been added to the build.
