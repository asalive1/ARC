# Dependency License Inventory

**Status:** Source dependency metadata reviewed; not a release approval.
**Inventory date:** 2026-11-25

## Scope and method

This snapshot covers the locked Rust workspace dependency graph and the
installed UI dependency graph from `ui/package-lock.json`. Rust metadata came
from `cargo metadata --locked --format-version 1`; npm packages were installed
with `npm ci` and read from package metadata. Target-specific Rust packages in
the lockfile are included. No package in either graph had a missing license
field.

This source-metadata snapshot does not itself include container base-image
operating-system packages, GStreamer/codec plugins, native audio backends, or
the Device Server SDK. Linux image contents have since been inventoried in
Docker Scout SPDX reports, summarized below; those inventories still require
package/license and notice review. This metadata review is not legal advice or
a signed-off release SBOM.

## Rust workspace

The locked graph contains 296 registry packages. License expressions reported
by their package metadata:

| SPDX expression | Packages |
|---|---:|
| MIT OR Apache-2.0 | 165 |
| MIT | 49 |
| Apache-2.0 OR MIT | 22 |
| Unicode-3.0 | 18 |
| MIT/Apache-2.0 | 10 |
| Zlib OR Apache-2.0 OR MIT | 3 |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 3 |
| Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 3 |
| Unlicense OR MIT | 2 |
| MIT OR Apache-2.0 OR LGPL-2.1-or-later | 2 |
| Apache-2.0 OR ISC OR MIT | 2 |
| ISC | 2 |
| BSD-3-Clause | 2 |
| Apache-2.0 OR BSL-1.0 OR MIT | 2 |
| CDLA-Permissive-2.0 | 2 |
| BSD-2-Clause OR Apache-2.0 OR MIT | 2 |
| Apache-2.0/MIT | 1 |
| Zlib | 1 |
| MIT AND BSD-3-Clause | 1 |
| Apache-2.0 AND ISC | 1 |
| Apache-2.0 OR BSL-1.0 | 1 |
| Apache-2.0 | 1 |
| Apache-2.0 WITH LLVM-exception | 1 |

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

Before a target image or release is approved, regenerate the source review
from locked manifests and review image SBOM/license data for the exact Linux
AMD64, Linux ARM64, and Windows x86-64 artifacts. Resolve the remaining
media-dependency and notice review in [open questions](open-questions.md) and
[licensing](licensing.md).

The user approved LGPL GStreamer components and the official
[Haivision SRT license](https://github.com/Haivision/srt/blob/master/LICENSE)
is MPL-2.0 for Phase 1, excluding GPL/AGPL media components and their
dependencies. Base OS packages are outside that exclusion but still require
package inventory, notices and legal review. GStreamer describes its framework
and newly written plugins as LGPL in its
[licensing FAQ](https://github.com/GStreamer/gstreamer/blob/main/subprojects/gst-docs/markdown/frequently-asked-questions/licensing.md).
The Phase 1 worker runtime has since been added. Its final image copies only
the required app, audioconvert, UDP, RTP, RTP-manager and SRT modules from
non-shipped Debian extraction stages rather than installing the full Base,
Good or Bad plugin bundles. AMD64 and emulated ARM64 audits confirmed the
required factories load, the selected plugin metadata is LGPL, and the known
GPL/unknown-plugin packages and unused DV/CACA dependencies are absent.

Docker Scout SPDX SBOMs were generated for the local Linux AMD64 API, UI,
PostgreSQL and worker images, plus the Linux ARM64 worker image. Each worker
SBOM contains 185 package records and 32 `NOASSERTION` license declarations
(28 distinct names); the API has 190/43, the UI 101/8, and PostgreSQL 80/11.
These are package inventories, not legal conclusions.

The API image SBOM has been regenerated from the current local image. Its
companion Rust CycloneDX SBOM contains 208 components from the locked API
dependency graph, including `argon2` 0.5.3, `blake2` 0.10.6 and
`password-hash` 0.5.0 for administrator authentication. Docker Scout does not
identify Rust crates statically linked into the API binary, so the two reports
are complementary, not a unified release SBOM. The reports are
[arc-api-linux-amd64.local.spdx.json](../sbom/arc-api-linux-amd64.local.spdx.json)
and [arc-api-rust.cdx.json](../sbom/arc-api-rust.cdx.json).

No tagged release, published release image, image-publishing workflow, or
Windows worker artifact is present in the repository. The following local
Linux/AMD64 image config IDs are build evidence only, not registry manifest
digests or release evidence:

| Local image | Config ID |
|---|---|
| `arc-api:latest` | `sha256:1fce4e6059333585dc532edab327ad15c11309cfc54f709e8364866004fa64ef` |
| `arc-ui:latest` | `sha256:b62dbe735d954377205a5925aa6e404c5f589108a992b354f1629e5c19fdcf6e` |
| `arc-worker:latest` | `sha256:d2d46084a1748499522e40b2f37ab0aa89175a67cb02169fd4610d564bfe270b` |

The API image SBOM's 43 `NOASSERTION` records span 36 names: `arc-api`,
`base-files`, `bash`, `curl`, `debian-archive-keyring`, `gcc-12`,
`gcc-12-base`, `glibc`, `gnutls28`, `gzip`, `krb5`, `libc6`, `libc-bin`,
`libcrypt1`, `libgcc-s1`, `libgcrypt20`, `libgnutls30`,
`libgssapi-krb5-2`, `libk5crypto3`, `libkrb5-3`, `libkrb5support0`,
`librtmp1`, `libselinux`, `libselinux1`, `libsemanage`, `libsemanage2`,
`libsemanage-common`, `libssh2`, `libssh2-1`, `libstdc++6`, `libtasn1-6`,
`libxcrypt`, `openssl`, `rtmpdump`, `tzdata` and `util-linux`. Debian copyright
files contain file-scoped, sometimes multiple license terms; the SBOM's
`NOASSERTION` values are therefore retained rather than replaced with
unsupported package-wide conclusions. These package notices and license
expressions still require review against the exact release images.
The `arc-api` package's declared workspace license is `MIT OR Apache-2.0`;
Docker Scout does not infer that expression from the statically linked binary.

For the retained AMD64 worker SBOM, the 32 `licenseDeclared: NOASSERTION`
records span 28 distinct names: `arc-worker`, `base-files`,
`debian-archive-keyring`, `gcc-12`, `gcc-12-base`, `glibc`, `gnutls28`,
`gstreamer1.0`, `libc-bin`, `libc6`, `libcrypt1`, `libgcc-s1`, `libgcrypt20`,
`libgnutls30`, `libgstreamer1.0-0`, `libselinux`, `libselinux1`,
`libsemanage`, `libsemanage-common`, `libsemanage2`, `libstdc++6`,
`libtasn1-6`, `libxcrypt`, `tzdata`, `bash`, `gzip`, `openssl` and
`util-linux`. The list includes Debian base/runtime packages and the GStreamer
core, plus the ARC image component; repeated SBOM records account for the
difference between 32 records and 28 names. These declarations are not evidence
that a package has no license or that its copyright notice is absent. They
must be reconciled against the relevant Debian source copyright files and ARC's
own workspace license before final-image sign-off. The base-OS GPL metadata is
outside the user's media-stack GPL exclusion, but still requires notice review.

Debian copyright metadata lists LGPL-2+ for selected GStreamer core and
UDP, RTP and RTP-manager source files. Runtime factory checks on AMD64 and
emulated ARM64 confirmed that app, audioconvert, coreelements, ALSA, UDP, RTP,
RTP-manager and SRT plugin metadata report LGPL. The SRT runtime package
identifies MPL-2.0 for its main code and also lists BSD-3-Clause, Zlib,
Unlicense and LGPL-2.1+ example-code notices.

The package-level SPDX declarations for `gstreamer1.0-alsa` and
`libgstreamer-plugins-base1.0-0` have broad GPL/LGPL alternatives even though
the required plugin modules report LGPL; their package-to-binary notice mapping
still needs review. The core `gstreamer1.0` and `libgstreamer1.0-0` SBOM
records retain `NOASSERTION`, although the parser source-to-library mapping
below is resolved. The `libunwind8` record includes GPL-2.0-only package
metadata and remains subject to base-OS notice review. Debian's GStreamer core
copyright metadata labels `gst/parse/grammar.tab.h` GPL-3+. The source build generates both
`grammar.tab.c` and `grammar.tab.h` with Bison; `parse.l` includes the header,
and `gst/meson.build` adds the parser outputs to the `gstreamer-1.0` library.
Debian's `libgstreamer1.0-0` package installs the resulting shared library,
not the generated header as a separate file. Reproducing the Debian source
generation with Bookworm Bison 3.8.2 confirmed the Bison exception notice in
both generated outputs. This matches the
[Bison exception conditions](https://www.gnu.org/software/bison/manual/html_node/Conditions.html)
and resolves the technical parser-generator and file-to-binary mapping; it
does not replace legal review of the full package and notice closure.

LGPL-2+ entries were confirmed for the selected ALSA (`ext/alsa`), app
(`gst-libs/gst/app`), audioconvert (`gst/audioconvert`) and RTP
(`gst-libs/gst/rtp`) source files; these source entries do not by themselves
resolve every package-level license declaration. The SRT plugin links to the
GnuTLS SRT build, not OpenSSL. The full Base plugin package and its
CD-paranoia plugin/dependency are absent from the narrowed runtime images.
These targeted checks do not constitute legal sign-off or close the full
transitive package review.
