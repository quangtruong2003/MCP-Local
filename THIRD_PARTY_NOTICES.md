# Third-Party Notices

This file lists the third-party components distributed with **Local Workspace
MCP Manager** and the licenses that apply to them. Runtime components that are
downloaded separately after installation (and therefore NOT included in the
NSIS installer) are marked as such.

---

## Lightpanda (runtime-provisioned, not bundled)

- **Component:** Lightpanda headless browser
- **Version:** 0.3.7 (exact, pinned)
- **License:** GNU Affero General Public License v3.0 (**AGPL-3.0**)
- **Upstream:** https://github.com/lightpanda-io/browser
- **Source reference:** `third-party/lightpanda-source-reference.txt`

The Lightpanda 0.3.7 binary is the **unmodified** upstream release artifact for
`x86_64-linux`, verified against its official SHA-256 digest before use. It is
kept as a separate process/runtime component and is downloaded on first browser
use rather than shipped inside the installer.

Under the AGPLv3, the complete corresponding source code for Lightpanda 0.3.7
is available from the upstream repository at
https://github.com/lightpanda-io/browser, and the license text is available at
https://www.gnu.org/licenses/agpl-3.0.txt.

## Managed Linux rootfs — Ubuntu 24.04 (runtime-provisioned, not bundled)

- **Component:** Minimal Ubuntu 24.04 (noble) root filesystem used to host the
  Lightpanda runtime under managed WSL2.
- **Image version:** `lwsl-ubuntu-noble-minbase-v1` (exact, pinned)
- **Base distribution:** Ubuntu 24.04 LTS ("Noble Numbat")

The rootfs is downloaded separately after installation and verified against a
pinned SHA-256 digest; it is **not** included in the NSIS installer.

Package inventory and licensing for the rootfs are provided as an SBOM and a
package manifest:

- **SBOM (SPDX-2.3):** `third-party/lightpanda-rootfs-sbom.spdx.json`
- **Package manifest:** `third-party/lightpanda-rootfs-packages.txt`

Ubuntu and its constituent packages are distributed under their respective
licenses; Canonical's Ubuntu licensing terms apply to the base distribution.

## Runtime manifest

The pinned runtime versions, asset URLs, and SHA-256 digests for both the
Lightpanda binary and the rootfs are recorded in
`src-tauri/resources/lightpanda-runtime-manifest.json` and enforced by
`scripts/verify-lightpanda-runtime-manifest.ps1` and
`scripts/verify-browser-release.ps1`.

## Tailscale funnel sidecar

- **Component:** Tailscale client binary used for the optional funnel sidecar.
- **License:** See `src-tauri/binaries/TAILSCALE-LICENSE.txt`.

## Frontend and build dependencies

Frontend runtime and build-time dependencies (React, Vite, Tailwind CSS, and
related packages) are declared in `package.json` and `package-lock.json` with
their respective licenses. They are compiled into the application bundle and do
not include any browser engine, Node.js runtime, Playwright, Chromium, or
Firefox.
