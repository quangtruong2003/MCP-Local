# Lightpanda Native MCP on Managed WSL2 — Design Specification

**Date:** 2026-08-27  
**Status:** Draft for user review  
**Target release:** Local Workspaces v0.2.0  
**Supersedes:** Playwright MCP + bundled Node/Chromium/Firefox browser runtime design  
**Primary platform:** Windows x86_64 with WSL2  
**Browser backend:** Lightpanda 0.3.7 stable, native MCP  
**Transport:** Private MCP JSON-RPC over stdio  
**Fallback browser:** None

---

## 1. Executive summary

Local Workspaces will replace its current Playwright MCP browser subsystem with Lightpanda running inside a dedicated, application-managed WSL2 distribution.

The new architecture removes Node.js, `@playwright/mcp`, `playwright`, `playwright-core`, bundled Chromium, bundled Firefox, Chrome/Edge/Firefox selection, Visible Debug mode, and any dependency on a graphical browser. Lightpanda becomes the only browser engine.

Lightpanda already embeds a native MCP server. Local Workspaces will launch `lightpanda mcp` directly inside WSL2 and communicate with it over private parent/child stdio. No MCP HTTP port is required for the primary runtime. No Playwright adapter or browser-download runtime is retained.

The WSL2 distribution is owned exclusively by Local Workspaces. The app must never install Lightpanda into, modify, reset, terminate, or unregister a user's personal Ubuntu, Debian, or other WSL distribution.

The application installer must not bundle WSL, a Linux root filesystem, Lightpanda, Node.js, Playwright, Chromium, or Firefox. Browser runtime provisioning happens lazily on first browser use. This makes the NSIS package small while keeping browser automation fully local after the first successful provision.

The current Local Workspaces `browser_*` MCP contract remains stable wherever technically meaningful. In addition, Local Workspaces adds safe first-class wrappers for useful native Lightpanda MCP capabilities that the existing 29-tool surface did not expose, including markdown, links, structured extraction, form detection, scrolling, and checkbox/radio state changes. Capabilities that fundamentally require a graphical rendering engine or browser features Lightpanda does not provide must fail deterministically with `BROWSER_FEATURE_UNSUPPORTED`; the app must never return fake success and must never silently fall back to Chrome or another browser.

---

## 2. Motivation

The current browser runtime bundles:

- a Windows Node.js runtime;
- `@playwright/mcp`;
- Playwright and Playwright Core;
- Playwright-managed Chromium;
- Playwright-managed Firefox;
- browser metadata and device descriptors.

The current staged browser runtime is approximately 1.15 GB before NSIS compression and is the dominant reason the release installer grew beyond 300 MB.

This is disproportionate to the product's actual browser workload. Local Workspaces is primarily an agentic automation product: agents need DOM state, semantic structure, JavaScript execution, form interaction, navigation, cookies, page text, console data, and deterministic control. They do not normally need a human-visible rendering pipeline.

Lightpanda is purpose-built for this workload. It runs JavaScript and browser Web APIs, maintains an in-memory DOM, exposes native MCP and CDP control surfaces, and intentionally omits the graphical rendering pipeline.

The replacement therefore optimizes for:

1. a dramatically smaller NSIS installer;
2. lower runtime memory and startup cost;
3. a shorter dependency chain;
4. direct reuse of Lightpanda's native MCP surface;
5. no Playwright or Chromium download lifecycle;
6. a private local transport;
7. predictable Windows installation through an app-owned WSL2 environment.

---

## 3. Product decisions

The following decisions are normative.

1. **Lightpanda is the only browser backend.**
   - No Chrome, Chromium, Edge, Firefox, WebKit, or Playwright fallback is permitted.
   - Old engine selectors are removed from the Settings UI.

2. **Lightpanda native MCP is the primary automation interface.**
   - Local Workspaces communicates directly with `lightpanda mcp`.
   - No Playwright MCP server exists in the new runtime.
   - No Node.js runtime is required.

3. **Windows runs Lightpanda through WSL2.**
   - Native Windows Lightpanda is not assumed.
   - The first supported release target is Windows x86_64 + WSL2.

4. **Local Workspaces owns a dedicated WSL distribution.**
   - It does not use the user's default WSL distro.
   - It does not modify any personal WSL distro.
   - It does not change the user's default distro.

5. **WSL is installed automatically when needed.**
   - First browser use calls an `ensure_runtime()` state machine.
   - If WSL is absent, Local Workspaces launches the Windows WSL installation flow using elevated `wsl.exe --install --no-distribution`.
   - If Windows requires a reboot, the provisioning state is persisted and resumes on the next app launch.

6. **Browser runtime is provisioned after installation, not included in NSIS.**
   - The NSIS bundle contains only the app, existing non-browser sidecars/resources, a small Lightpanda runtime manifest, and license/notice metadata.
   - A minimal Linux rootfs and Lightpanda are downloaded only when Browser Automation is first used.

7. **The browser remains background-only.**
   - There is no Visible Debug mode.
   - There is no DevTools window.
   - Browser operations must not create a visible console window.

8. **Real screenshots are unsupported.**
   - Lightpanda has no graphical rendering pipeline.
   - `browser_screenshot` must return `BROWSER_FEATURE_UNSUPPORTED`.
   - Placeholder images from Lightpanda must never be returned as if they were real screenshots.

9. **Pause remains the top-level kill switch.**
   - Pause immediately denies new browser operations.
   - Pause terminates all active Lightpanda child processes.

10. **Browser Automation OFF remains authoritative.**
    - It denies browser operations.
    - It terminates all active Lightpanda processes.
    - `browser_get_status` remains readable while OFF.

11. **No fake compatibility.**
    - Unsupported features fail with stable structured errors.
    - The app must never silently switch engines, invent visual state, or claim success for an operation the engine did not perform.

---

## 4. External technical baseline

The implementation is based on the following verified external behavior as of 2026-08-27:

- Lightpanda stable release baseline: `0.3.7`.
- Lightpanda has no native Windows binary; Windows support is through WSL2.
- Lightpanda ships a single native executable.
- `lightpanda mcp` exposes MCP JSON-RPC 2.0 over stdio.
- Lightpanda 0.3.7 also supports MCP HTTP sessions, but Local Workspaces v0.2.0 intentionally does not use that network transport.
- `lightpanda mcp` exposes navigation, semantic tree/DOM reading, element discovery, click/fill/hover/press/select/check, JavaScript evaluation, waits, cookies, console logs, structured extraction, and search.
- Lightpanda supports persistent storage via `--storage-engine sqlite` and `--storage-sqlite-path`.
- Lightpanda supports cookie load/save paths.
- Lightpanda telemetry can be disabled with `LIGHTPANDA_DISABLE_TELEMETRY=true`.
- Lightpanda crash core dumps can be disabled with `LIGHTPANDA_DISABLE_CORE_DUMP`.
- Lightpanda does not render real pixels and cannot provide genuine screenshots.
- Windows WSL supports installation without a default distro via `wsl --install --no-distribution`.
- Windows WSL supports custom distro import with `wsl --import <Distro> <InstallLocation> <Tar> --version 2`.

These facts are release assumptions. Runtime startup must still validate the installed Lightpanda binary and required MCP tools instead of trusting a version string alone.

---

## 5. Version and supply-chain policy

### 5.1 Lightpanda version

Local Workspaces v0.2.0 pins:

```text
Lightpanda version: 0.3.7
Platform asset: lightpanda-x86_64-linux
Expected SHA-256:
895339b02205171a181dde743ae0068bb4564884076feac8482baca9c212aa5a
```

The implementation must not use the floating `nightly` release.

A later Local Workspaces release may update Lightpanda only by updating an embedded runtime manifest and passing the full runtime compatibility suite.

### 5.2 Runtime manifest

The desktop app includes a small immutable manifest similar to:

```json
{
  "schema_version": 1,
  "runtime_family": "lightpanda-wsl",
  "lightpanda_version": "0.3.7",
  "lightpanda_arch": "x86_64-linux",
  "lightpanda_sha256": "895339b02205171a181dde743ae0068bb4564884076feac8482baca9c212aa5a",
  "distro_image_version": "lwsl-ubuntu-noble-minbase-v1",
  "distro_arch": "amd64"
}
```

The release build supplies the exact distro image URL and SHA-256 through the generated manifest.

### 5.3 Runtime update policy

Runtime updates are app-controlled, not self-updating.

- Never call `lightpanda version --check` automatically.
- Never install `nightly`.
- Never update Lightpanda while a browser session is active.
- Install a new Lightpanda version side-by-side.
- Verify it before switching the `current` symlink.
- Keep the immediately previous verified version until the new version successfully completes a smoke launch.
- If verification fails, remain on the previous version and expose a repair/update error in desktop status.

### 5.4 Telemetry and dumps

Every Lightpanda process must receive:

```text
LIGHTPANDA_DISABLE_TELEMETRY=true
LIGHTPANDA_DISABLE_CORE_DUMP=1
```

Production must use `--log-level warn`.

Debug builds may opt into `info` or `debug` through a developer-only mechanism, never through model-controlled MCP input.

---

## 6. Managed WSL2 architecture

### 6.1 Distro identity

Preferred distro name:

```text
LocalWorkspaces-Lightpanda
```

The actual registered name is persisted because a name collision may exist.

If `LocalWorkspaces-Lightpanda` already exists:

1. inspect the app ownership marker inside it;
2. if the marker identifies the current Local Workspaces runtime, reuse it;
3. if it is not owned by Local Workspaces, do not modify or unregister it;
4. allocate a collision-safe name such as:

```text
LocalWorkspaces-Lightpanda-<8-char-install-id>
```

### 6.2 Ownership marker

An app-owned distro contains:

```text
/etc/local-workspaces-runtime.json
```

with at least:

```json
{
  "owner": "com.local.workspace-mcp-manager",
  "runtime": "lightpanda",
  "install_id": "<host-generated-install-id>",
  "schema_version": 1
}
```

The host persists the same random `install_id` beside the managed distro metadata under LocalAppData. A distro is considered owned only when its marker matches the host `install_id` and its registered install location matches the app-owned runtime directory.

No destructive WSL action is allowed unless all ownership checks pass.

### 6.3 Install location

The WSL VHD/runtime must live under LocalAppData, not Roaming AppData:

```text
%LOCALAPPDATA%\Local Workspace MCP Manager\browser\wsl\<distro-id>\
```

Large browser runtime state must never be stored in a roaming profile.

### 6.4 Minimal distro image

The app uses a project-owned, separately downloaded, minimal glibc-based WSL rootfs artifact.

The release pipeline builds this artifact from Ubuntu 24.04 LTS minbase and includes only the runtime packages required to:

- boot under WSL2;
- run POSIX shell commands required by provisioning;
- resolve DNS;
- make HTTPS connections;
- validate TLS certificates;
- execute the pinned Lightpanda binary.

The distro image must not contain:

- Node.js;
- npm;
- Playwright;
- Chromium;
- Firefox;
- a desktop environment;
- systemd services not required by the runtime;
- developer toolchains.

The rootfs is not part of NSIS. It is a first-use runtime download with a pinned checksum.

### 6.5 WSL isolation configuration

Provisioning writes `/etc/wsl.conf` so the managed distro does not expose unnecessary Windows integration:

```ini
[automount]
enabled=false

[interop]
enabled=false
appendWindowsPath=false
```

The distro therefore does not automatically mount Windows drives and cannot launch Windows executables from Linux.

Browser internet access remains enabled.

### 6.6 Linux runtime user

Lightpanda must not run as root.

Provisioning creates an unprivileged account:

```text
user: localworkspaces
home: /var/lib/local-workspaces
shell: /bin/sh
```

Owned directories include:

```text
/var/lib/local-workspaces/browser/
/var/lib/local-workspaces/browser/profile/
/var/lib/local-workspaces/browser/cookies/
/var/lib/local-workspaces/browser/cache/
/var/lib/local-workspaces/browser/artifacts/
```

Executable installation is root-owned under:

```text
/opt/local-workspaces/lightpanda/<version>/lightpanda
/opt/local-workspaces/lightpanda/current -> <version>
```

---

## 7. WSL provisioning state machine

Provisioning is idempotent and serialized by one process-wide lock.

### 7.1 States

```text
uninitialized
checking_host
wsl_install_required
installing_wsl
reboot_required
distro_install_required
installing_distro
lightpanda_install_required
installing_lightpanda
verifying_runtime
ready
repair_required
unsupported_host
```

Only `ready` may launch browser sessions.

### 7.2 First-use flow

`browser_open` calls `ensure_runtime()` before creating a page.

#### Case A — WSL already healthy

1. validate WSL;
2. validate/reuse app-owned distro;
3. validate Lightpanda version/digest/tool surface;
4. provision missing components if needed;
5. launch browser.

#### Case B — WSL installed but managed distro absent

1. download pinned rootfs to a host staging directory;
2. verify SHA-256 before import;
3. import with WSL2;
4. configure isolation and runtime user;
5. install Lightpanda;
6. verify;
7. launch browser.

#### Case C — WSL absent

1. launch an elevated Windows process that runs:

```text
wsl.exe --install --no-distribution
```

2. persist provisioning state;
3. if Windows reports a reboot is required, return `BROWSER_WSL_REBOOT_REQUIRED`;
4. after reboot and next Local Workspaces launch, resume automatically from host validation;
5. install the managed distro and Lightpanda;
6. continue the originally requested browser setup on the next browser call.

The application must not attempt to bypass UAC.

### 7.3 Interrupted provisioning

All downloads use `.partial` files and atomic rename after checksum validation.

A crash at any stage must be recoverable:

- invalid partial downloads are discarded;
- an incomplete imported distro without a valid ownership marker is never treated as ready;
- Lightpanda installation uses versioned directories and atomic `current` symlink replacement;
- `ready` is persisted only after the MCP smoke test passes.

### 7.4 Repair

Desktop Settings exposes **Repair Browser Runtime**.

Repair:

1. stops all browser children;
2. revalidates WSL and the ownership marker;
3. rechecks runtime manifest, binary digest, permissions, version, and MCP tools;
4. replaces only app-owned runtime files;
5. rebuilds the app-owned distro only when its integrity cannot be repaired in place.

Repair must never touch other WSL distros.

---

## 8. Lightpanda installation into WSL

### 8.1 Host download

The Windows host downloads the pinned Lightpanda asset to a staging cache.

The file must be hashed before it is copied into WSL.

A digest mismatch is fatal and produces:

```text
BROWSER_RUNTIME_INTEGRITY_FAILED
```

The mismatched binary is deleted and never executed.

### 8.2 Copy without Windows-drive automount

Because the managed distro disables Windows drive automount, provisioning must not depend on `/mnt/c`.

The host streams the already-verified binary through stdin to an app-owned command inside WSL, for example a root-owned `cat`/install operation that writes the versioned target file.

No shell fragment may contain user/model-controlled data.

All paths and version values used in this operation come from application constants or the signed/pinned runtime manifest.

### 8.3 Verification

After install:

1. run the binary as `localworkspaces`;
2. verify `lightpanda version` reports exactly `0.3.7`;
3. launch a temporary `lightpanda mcp` child;
4. perform MCP initialize;
5. run `tools/list`;
6. verify the required native tool set;
7. terminate the smoke child;
8. atomically switch `current`.

Required v0.2.0 native tools:

```text
goto
getUrl
getCookies
consoleLogs
tree
markdown
html
nodeDetails
findElement
interactiveElements
links
extract
structuredData
search
click
hover
fill
selectOption
setChecked
press
scroll
detectForms
evaluate
waitForSelector
waitForState
waitForScript
```

Additional upstream tools are allowed but are not automatically exposed to ChatGPT.

Missing required tools cause:

```text
BROWSER_RUNTIME_VERSION_MISMATCH
```

---

## 9. Runtime process architecture

```text
ChatGPT / MCP client
        |
        | Local Workspaces MCP
        v
+--------------------------------+
| WorkspaceServer (Rust / rmcp)  |
| - stable browser_* schemas     |
| - Pause / policy               |
| - audit                        |
| - validation                   |
+---------------+----------------+
                |
                v
+--------------------------------+
| BrowserManager                 |
| - runtime ensure/provision     |
| - page/session state           |
| - ref generations              |
| - artifact/cookie state        |
+---------------+----------------+
                |
          private child stdio
                |
                v
+--------------------------------+
| wsl.exe child                  |
| - specific managed distro      |
| - user localworkspaces         |
+---------------+----------------+
                |
                v
+--------------------------------+
| Lightpanda native MCP          |
| lightpanda mcp                 |
| - JSON-RPC 2.0 over stdio      |
| - one logical page process     |
+--------------------------------+
```

### 9.1 No listening browser port

Production v0.2.0 uses stdio only.

Do not start:

- Lightpanda MCP HTTP transport;
- CDP server;
- local browser TCP listeners;
- WebSocket listeners.

This preserves the private parent-child containment model and removes a local cross-process attack surface.

### 9.2 Logical tabs

Lightpanda stdio provides one default browsing session per process.

Local Workspaces preserves its `browser_tabs` contract by representing each logical tab as a separate Lightpanda MCP child.

Each logical page owns:

```text
page_id
child process
MCP client
current URL/title
ref generation
ref map
console ring
network/performance cache
cookie-jar path
```

The active page is the child receiving page-scoped commands.

This design intentionally prefers several lightweight Lightpanda processes over one Chromium process with tabs.

### 9.3 Process containment

On Windows:

- spawn `wsl.exe` with no visible console window;
- attach every WSL child to the existing Windows Job Object containment strategy where supported;
- retain child PIDs for diagnostics;
- dropping/closing a page terminates its child;
- closing the browser terminates all page children;
- Pause terminates all page children immediately.

When the final page child is closed, Local Workspaces may run:

```text
wsl.exe --terminate <owned-distro-name>
```

to release WSL memory.

The command is allowed only for the verified app-owned distro.

---

## 10. Profile, cookies, and persistent storage

### 10.1 Persistent profile root

Inside WSL:

```text
/var/lib/local-workspaces/browser/profile/
```

### 10.2 Web Storage

Use:

```text
--storage-engine sqlite
--storage-sqlite-path /var/lib/local-workspaces/browser/profile/storage.sqlite
```

The implementation must validate concurrent access with multiple logical page processes.

Release acceptance requires the multi-page storage concurrency test to pass without corruption.

### 10.3 HTTP cache

Use an app-owned cache:

```text
--http-cache-dir /var/lib/local-workspaces/browser/cache/
```

Cache cleanup follows bounded size/age rules; it is not allowed to grow without limit.

### 10.4 Cookies

Maintain a canonical app-owned cookie snapshot plus one exit jar per page.

Every page process launches with:

```text
--cookie <canonical-cookie-json-path>
--cookie-jar <page-specific-exit-json-path>
```

After navigation and meaningful page interactions, Local Workspaces may call `getCookies(all=true)` and merge the result into the canonical cookie store.

Before a newly created logical tab starts, the canonical cookie store is refreshed from the active page so a newly opened tab inherits current login cookies.

Known v0.2.0 limitation:

- cookies newly created in one already-running logical tab are not guaranteed to appear live in another already-running logical tab until that tab is restarted;
- Local Workspaces must document this rather than claiming exact Chromium shared-context semantics.

### 10.5 Reset Browser Data

Desktop-only **Reset Browser Data**:

1. closes all page children;
2. removes only `/var/lib/local-workspaces/browser/profile`, cookies, cache, and browser artifacts inside the verified owned distro;
3. recreates clean directories with the correct owner;
4. does not remove the distro or Lightpanda binary.

---

## 11. Stable Local Workspaces browser contract

The existing 29 Local Workspaces browser tools remain registered so existing agent guidance and clients do not break at discovery time. v0.2.0 adds 10 safe native-first wrappers, bringing the browser surface to 39 Local Workspaces tools.

Each tool is classified as:

- **Native** — direct Lightpanda MCP mapping;
- **Adapter** — implemented by Local Workspaces using multiple native MCP primitives or app-owned page JavaScript;
- **Degraded** — returns useful non-visual data with explicitly reduced semantics;
- **Unsupported** — stable tool exists but returns `BROWSER_FEATURE_UNSUPPORTED`.

### 11.1 Compatibility matrix

| Local Workspaces tool | v0.2.0 status | Lightpanda implementation |
|---|---|---|
| `browser_get_status` | Adapter | Manager/runtime state |
| `browser_open` | Native + adapter | ensure runtime + spawn child + `goto` |
| `browser_close` | Adapter | close all children |
| `browser_navigate` | Native + adapter | `goto`; app-owned `history.back/forward`, reload script |
| `browser_search` | Native | `search` |
| `browser_snapshot` | Adapter | `tree` + opaque ref normalization |
| `browser_click` | Native + adapter | ref -> `nodeDetails` selector -> `click` |
| `browser_hover` | Native + adapter | ref -> `nodeDetails` selector -> `hover` |
| `browser_fill` | Native + adapter | ref -> `nodeDetails` selector -> `fill` |
| `browser_select` | Native + adapter | ref -> `nodeDetails` selector -> `selectOption` |
| `browser_press` | Native | `press` |
| `browser_drag` | Adapter | safe app-owned synthetic HTML5 drag sequence |
| `browser_wait` | Native + adapter | `waitForSelector`, `waitForState`, `waitForScript`, host delay |
| `browser_evaluate` | Native | `evaluate` in page context |
| `browser_dom` | Adapter | `nodeDetails` + `html` + bounded computed-style script |
| `browser_console` | Native + adapter | `consoleLogs` into Local Workspaces ring/cursor model |
| `browser_network` | Degraded | Performance Resource Timing metadata; no bodies |
| `browser_performance` | Degraded | Navigation/Resource Performance API; no renderer/CDP metrics |
| `browser_trace` | Adapter | Local Workspaces trace format, not Playwright trace |
| `browser_tabs` | Adapter | one stdio Lightpanda child per logical tab |
| `browser_run_steps` | Adapter | existing validated Local Workspaces step dispatcher |
| `browser_set_viewport` | Unsupported | no real rendering/layout viewport |
| `browser_emulate_device` | Unsupported | no truthful graphical/mobile emulation |
| `browser_screenshot` | Unsupported | no renderer; never return placeholder |
| `browser_devtools` | Unsupported/status | no visible DevTools/browser UI |
| `browser_dialog` | Unsupported | no required native MCP primitive in pinned baseline |
| `browser_upload` | Unsupported | native file upload not provided in pinned baseline |
| `browser_downloads` | Unsupported | no managed download artifact primitive |
| `browser_save_download_to_workspace` | Unsupported | depends on managed browser downloads |

Unsupported calls must not partially perform side effects before returning the error.

### 11.2 New native-first Local Workspaces tools

To take advantage of Lightpanda instead of merely emulating the old Playwright surface, v0.2.0 adds the following safe wrappers.

#### `browser_markdown`

Arguments:

```text
url?: string
ref?: string
max_bytes?: integer
timeout_ms?: integer
```

Behavior:

- optional `url` is validated by normal navigation policy;
- optional `ref` resolves to a backend node and scopes output to that subtree;
- maps to Lightpanda `markdown`;
- enforces the existing bounded-output policy plus `max_bytes`;
- returns token-efficient Markdown.

#### `browser_html`

Arguments:

```text
url?: string
ref?: string
timeout_ms?: integer
```

Behavior:

- maps to Lightpanda `html`;
- returns document HTML or one ref-scoped node's `outerHTML`;
- keeps sensitive-value redaction rules where Local Workspaces applies them;
- output is byte-bounded.

#### `browser_links`

Arguments:

```text
url?: string
timeout_ms?: integer
```

Behavior:

- maps to Lightpanda `links`;
- returns normalized visible text, absolute `href`, and Local Workspaces opaque ref when a backend node ID is present;
- URLs are treated as page data, not automatically navigated.

#### `browser_find_element`

Arguments:

```text
role?: string
name?: string
```

Behavior:

- maps to Lightpanda `findElement`;
- requires at least one of `role` or `name`;
- converts every returned backend node ID into a current-generation Local Workspaces ref;
- returns role/name/selector metadata only when supplied safely by Lightpanda.

#### `browser_interactive_elements`

Arguments:

```text
url?: string
timeout_ms?: integer
```

Behavior:

- maps to Lightpanda `interactiveElements`;
- returns a bounded flat list of actionable elements;
- converts backend node IDs into opaque refs;
- is recommended when the agent needs a small action-oriented representation rather than a full semantic tree.

#### `browser_extract`

Arguments:

```text
schema: object
```

Local Workspaces accepts a JSON object instead of requiring the model to manually encode Lightpanda's upstream JSON-string argument.

Behavior:

- validate maximum schema depth, selector count, and encoded size;
- serialize the validated object once inside the adapter;
- map to Lightpanda `extract`;
- return structured JSON;
- never enable upstream `save` bridge state from model input.

The schema supports Lightpanda's safe selector shapes:

```text
"field": "<selector>"
"field": ["<selector>"]
"field": {"selector":"<selector>","attr":"<attribute>"}
"field": [{"selector":"<selector>","attr":"<attribute>","limit":N}]
"field": [{"selector":"<selector>","fields":{...},"limit":N}]
```

#### `browser_structured_data`

Arguments:

```text
url?: string
timeout_ms?: integer
```

Behavior:

- maps to `structuredData`;
- returns available JSON-LD, OpenGraph, Twitter-card, HTML metadata, and canonical/link metadata;
- output is byte-bounded.

#### `browser_detect_forms`

Arguments:

```text
url?: string
timeout_ms?: integer
```

Behavior:

- maps to `detectForms`;
- returns form/field/type/required structure;
- does not expose password field values.

#### `browser_scroll`

Arguments:

```text
ref?: string
x?: integer
y?: integer
```

Behavior:

- maps to Lightpanda `scroll`;
- with `ref`, scrolls the corresponding backend node;
- without `ref`, scrolls the page;
- coordinates represent Lightpanda's logical scroll state, not physical screen pixels.

#### `browser_set_checked`

Arguments:

```text
ref: string
checked?: bool = true
```

Behavior:

- resolves the Local Workspaces ref to its backend node, then calls `nodeDetails` to recover the current CSS selector;
- maps that selector to `setChecked` (Lightpanda 0.3.7 action tools do not accept `backendNodeId` directly);
- supports checkbox and radio controls;
- invalidates refs conservatively after the action.

### 11.3 Native tools intentionally kept internal

The following Lightpanda native MCP tools are not directly exposed to the model:

```text
getEnv
getCookies
save
session_new
session_list
session_close
```

Reasons:

- `getEnv` can expose process configuration and is unnecessary for browser automation;
- `getCookies` is used internally for profile persistence and may contain sensitive session credentials;
- `save` writes PandaScript and creates a process-execution-adjacent artifact path that is outside v0.2.0's page-only execution model;
- `session_*` applies to Lightpanda HTTP transport, while Local Workspaces deliberately uses private stdio and owns logical page lifecycle itself.

This policy follows least privilege: use all safe native browser capabilities, not every upstream administrative/debug primitive.

---

## 12. Navigation correctness

Lightpanda's `goto` response alone is not sufficient proof of successful navigation.

`browser_open(url)` and `browser_navigate(url)` must:

1. validate the URL with the existing Local Workspaces HTTP/HTTPS policy;
2. call `goto`;
3. perform a follow-up state/content probe using `getUrl` and a bounded `tree` or `markdown` call;
4. map resolution/network/load errors from the verification call to a stable Local Workspaces navigation error.

Do not report navigation success solely because `goto` returned a success string.

Back, forward, and reload use fixed application-owned JavaScript only:

```text
history.back()
history.forward()
location.reload()
```

The model never supplies these scripts.

After navigation, refs are invalidated.

---

## 13. Snapshot and element reference model

### 13.1 Snapshot source

`browser_snapshot` calls Lightpanda `tree`.

The response is normalized into a Local Workspaces bounded semantic snapshot.

### 13.2 Opaque refs

Lightpanda backend node IDs must not become the durable public identity.

Local Workspaces creates refs such as:

```text
e1
e2
e3
```

and stores:

```text
(ref, page_id, generation) -> backendNodeId
```

### 13.3 Generation invalidation

A page has a monotonically increasing ref generation.

Increment generation and clear old refs after:

- navigation;
- reload;
- tab restart;
- operations known to replace major DOM state;
- any upstream error indicating a node no longer exists.

For safety, side-effecting interactions may invalidate refs conservatively.

Using an old ref returns:

```text
BROWSER_STALE_REF
```

and agent guidance instructs the caller to take a new snapshot.

### 13.4 Selector recovery

For adapter operations requiring a CSS selector, Local Workspaces calls `nodeDetails` for the backend node and uses Lightpanda's returned selector.

Selectors are treated as data and JSON-encoded when inserted into fixed application-owned JavaScript.

---

## 14. Interaction semantics

### 14.1 Native interactions

Lightpanda 0.3.7 action tools use CSS selectors, not `backendNodeId` values. For every ref-based native interaction, Local Workspaces resolves the opaque ref to its current backend node, calls `nodeDetails`, and passes the returned CSS selector to the native action:

```text
browser_click  -> ref -> nodeDetails.selector -> click
browser_hover  -> ref -> nodeDetails.selector -> hover
browser_fill   -> ref -> nodeDetails.selector -> fill
browser_select -> ref -> nodeDetails.selector -> selectOption
browser_press  -> press
```

Selector recovery is an adapter responsibility even though the final action itself is native.

### 14.2 Drag

Pinned Lightpanda native MCP does not expose a drag tool.

Local Workspaces may preserve `browser_drag` for HTML5 drag/drop by:

1. resolving start/end refs to selectors;
2. running a fixed application-owned script;
3. creating a `DataTransfer`;
4. dispatching `dragstart`, `dragenter`, `dragover`, `drop`, and `dragend`.

This is DOM-event drag semantics, not pixel-accurate pointer dragging.

If the target page requires real rendered coordinates, return:

```text
BROWSER_FEATURE_UNSUPPORTED
```

### 14.3 No coordinate API

Do not add a model-facing coordinate click/drag API in this release.

The agent is expected to use semantic snapshot refs.

---

## 15. Wait semantics

Existing `browser_wait` conditions remain.

### `text_present`

Use `waitForScript` with a fixed function over `document.body.innerText`.

### `text_absent`

Same, negated.

### `ref_visible`

Resolve ref -> selector and use `waitForSelector` plus a bounded visibility predicate where required.

### `ref_hidden`

Use `waitForScript` with a fixed selector predicate.

Because Lightpanda lacks real layout, visibility means DOM/CSS visibility available to Lightpanda, not pixel visibility.

### `url_contains`

Use `waitForScript` over `location.href`.

### `load_state`

Preserve the existing Local Workspaces public values exactly:

```text
loading
interactive
complete
```

Do not replace them with Lightpanda's native state names because that would break the existing `browser_wait` schema. Implement the compatibility mapping with fixed application-owned waits:

```text
loading     -> waitForScript(document.readyState === "loading")
interactive -> waitForScript(document.readyState === "interactive")
complete    -> waitForState("load") with a follow-up readyState verification when needed
```

Lightpanda-native states such as `domcontentloaded` and `networkidle` remain internal implementation details in v0.2.0 and are not new model-facing enum values.

### `delay`

Use host-side `tokio::time::sleep` in bounded polling intervals so Pause can interrupt the wait.

All supplied text/selector values are encoded as JSON string literals; never construct executable JavaScript by raw concatenation.

---

## 16. DOM inspection

`browser_dom(ref)` returns a normalized structure assembled from:

- `nodeDetails`;
- `html` for the node;
- a fixed page-context script for a small allowlist of computed-style properties.

Sensitive fields remain redacted under the existing Local Workspaces rules.

The returned structure may include:

```text
tagName
role
accessibleName
attributes
state
selector
outerHTML
style.display
style.visibility
style.position
style.color
style.backgroundColor
style.fontSize
```

Do not claim accurate pixel geometry.

Any `rect`, x/y, or rendered layout fields from the old implementation must be removed or explicitly `null`/`approximate=false` rather than fabricated.

---

## 17. Console model

Lightpanda `consoleLogs` returns buffered messages and clears its upstream buffer.

Local Workspaces maintains its own bounded ring per page so the existing cursor model remains usable.

Flow:

1. call `consoleLogs`;
2. append normalized messages to the page ring;
3. assign monotonically increasing local sequence IDs;
4. apply requested level filtering;
5. return the requested bounded slice and next cursor.

The ring must have a hard count and byte limit.

Console messages are page data and must never be written to the permanent audit log.

---

## 18. Network model

Pinned Lightpanda native MCP does not expose a Local Workspaces-equivalent request-event tool.

`browser_network` remains available in degraded mode using page Performance APIs.

A fixed application-owned evaluation reads:

```text
performance.getEntriesByType("navigation")
performance.getEntriesByType("resource")
```

Normalize fields such as:

```text
url
initiator_type
start_time
duration
transfer_size
encoded_body_size
decoded_body_size
response_status   // when supported, otherwise null
source = "performance_api"
```

Rules:

- never fetch response bodies;
- never return Authorization, Cookie, or secret-bearing headers;
- apply the existing regex URL filter locally;
- keep the existing bounded cursor/ring behavior where practical;
- explicitly report `coverage: "completed-resource-timing"` so callers know this is not a full CDP network event stream.

This degraded mode must not be described as full DevTools network inspection.

---

## 19. Performance model

`browser_performance` returns Lightpanda-appropriate metrics:

```text
engine: "lightpanda"
navigation: [...]
resources: [...]
resource_count
total_transfer_size_when_available
runtime_notes
```

It does not return fake Chromium CDP `Performance.getMetrics` values.

Rendering metrics such as paint/compositing/layout fidelity are unavailable.

If the existing response schema has optional Chromium-specific fields, return them as `null` with a capability flag rather than manufacturing values.

---

## 20. Trace model

Playwright tracing is removed.

`browser_trace` remains as a Local Workspaces-owned diagnostic trace.

### Start

Begin recording:

- local timestamp;
- page ID;
- tool name;
- sanitized argument metadata;
- start/end/result class;
- URL before/after;
- navigation transitions;
- console snapshots;
- network/performance snapshots;
- process restart/crash events.

### Stop

Write an artifact under the existing Local Workspaces browser artifact sandbox:

```text
trace.jsonl
trace-manifest.json
```

The manifest states:

```text
format = "local-workspaces-lightpanda-trace-v1"
screenshots = false
dom_snapshots = false
playwright_compatible = false
```

No cookie values, authorization data, page passwords, or full response bodies are allowed in traces.

`browser_trace(status)` reports whether this Local Workspaces trace recorder is active.

---

## 21. Tabs and logical pages

The existing tool surface:

```text
browser_tabs(action=list|new|select|close)
```

remains.

### List

Return Local Workspaces page IDs, current URL/title, and active status.

### New

1. refresh canonical cookies from the active page;
2. spawn a new Lightpanda stdio child;
3. assign a fresh `page_id`;
4. optionally navigate to the supplied URL;
5. make it active.

### Select

Change the active page binding only.

### Close

Terminate that Lightpanda child, merge its cookie jar, remove refs/buffers, and choose a deterministic remaining active page.

Page IDs remain opaque and session-scoped.

---

## 22. Unsupported visual and file capabilities

### 22.1 Screenshot

`browser_screenshot` returns:

```text
BROWSER_FEATURE_UNSUPPORTED
```

with safe explanation:

```text
Lightpanda does not provide real rendered screenshots.
Use browser_snapshot/browser_dom for semantic page state.
```

Do not call Lightpanda screenshot APIs that return placeholders.

### 22.2 DevTools

`browser_devtools` is read-only and returns structured support:

```json
{
  "supported": false,
  "visible_debug_supported": false,
  "reason": "lightpanda_has_no_graphical_devtools_ui"
}
```

It never launches anything.

### 22.3 Viewport and device emulation

`browser_set_viewport` and `browser_emulate_device` return `BROWSER_FEATURE_UNSUPPORTED`.

No fake viewport success is allowed.

### 22.4 File upload

`browser_upload` returns `BROWSER_FEATURE_UNSUPPORTED` before staging any workspace file.

This is important: an unsupported call must not read or copy user files unnecessarily.

### 22.5 Downloads

`browser_downloads` and `browser_save_download_to_workspace` return `BROWSER_FEATURE_UNSUPPORTED`.

No ordinary HTTP fetch should be relabeled as a browser download because that would change cookie/navigation semantics.

### 22.6 JavaScript dialogs

`browser_dialog` returns `BROWSER_FEATURE_UNSUPPORTED` in v0.2.0.

No automatic `alert/confirm/prompt` monkeypatch is installed because that can alter application behavior.

---

## 23. `browser_run_steps`

The batch tool remains an orchestration layer over the same public Local Workspaces handlers.

Rules do not change:

- at most 20 steps;
- validate every step before execution;
- execute sequentially;
- stop on first failure;
- never replay a side-effecting step after process loss;
- each step goes through Pause, Browser Automation, URL, ref, and policy checks.

If a batch contains an unsupported step such as screenshot or viewport, stop at that step with `BROWSER_FEATURE_UNSUPPORTED`.

---

## 24. Settings schema and migration

### 24.1 New product-visible settings

Keep:

```text
browser_automation_enabled: bool
```

Remove from UI:

```text
browser_engine selector
browser_run_mode selector
```

The browser section displays the fixed backend:

```text
Lightpanda (WSL2)
Background only
```

### 24.2 Legacy settings migration

Existing serialized settings may contain:

```text
browser_engine =
  bundled_chromium | chrome | edge | firefox

browser_run_mode =
  background | visible_debug
```

On schema migration:

- any legacy browser engine maps to `lightpanda`;
- any legacy run mode maps to `background`;
- no legacy engine binary is launched after migration;
- invalid/unknown legacy values use the same deterministic Lightpanda/background defaults.

### 24.3 Internal status enums

The runtime status model may retain engine fields for MCP compatibility, but the only emitted current engine is:

```text
lightpanda
```

`actual_engine` must also be `lightpanda` when running.

### 24.4 Browser status additions

`browser_get_status` should include:

```text
runtime_backend
runtime_state
lightpanda_version
wsl_installed
wsl_version_when_available
managed_distro_name
managed_distro_ready
reboot_required
runtime_repair_required
capabilities
```

No absolute Windows or WSL filesystem paths are returned.

---

## 25. Desktop Settings UX

The Browser Automation section becomes:

```text
Browser Automation      [On/Off]

Backend
Lightpanda (WSL2)

Runtime status
Ready / Setup required / Installing / Restart required / Repair required

Lightpanda
0.3.7

Capabilities
Semantic browser automation
No visual screenshots

[Set up / Repair runtime]
[Reset browser data]
[Remove browser runtime]
```

### 25.1 Setup behavior

When Browser Automation is ON but runtime is absent:

- do not download it merely because Settings opened;
- provision lazily on first `browser_open` or when the user presses Setup;
- show progress in desktop state;
- if UAC/reboot is required, show the exact state.

### 25.2 Remove browser runtime

Desktop-only action:

1. close all browser processes;
2. verify distro ownership marker;
3. unregister only the owned distro;
4. remove the app-owned WSL install directory and cached runtime artifacts;
5. preserve Browser Automation toggle state;
6. next browser use provisions again.

This operation must never be exposed as a model-callable MCP tool.

---

## 26. Browser policy integration

All existing browser policy rules remain authoritative.

### Before any page operation

Check in order:

1. Pause;
2. Browser Automation enabled;
3. runtime availability/provisioning state;
4. browser session/page state;
5. request-specific validation.

Pause keeps deterministic highest priority.

### URL policy

Only validated HTTP(S) navigation is model-callable.

Disallow:

```text
file:
data:
javascript:
chrome:
edge:
about:
wsl:
```

unless already covered by stricter existing policy.

### Page JavaScript

`browser_evaluate` remains page-context JavaScript only.

It must not execute shell, WSL, Rust-process, or Lightpanda agent/PandaScript code.

Local Workspaces must never expose Lightpanda's script-saving/execution capability as a path to process-level code execution.

---

## 27. Audit and privacy

### 27.1 Audit

Audit records browser operations using the existing fail-closed policy.

Audit metadata may include:

```text
tool name
outcome
session/page opaque IDs
URL host / normalized URL according to existing policy
duration
error code
```

Do not audit:

- filled text;
- JavaScript source;
- cookie values;
- console message bodies;
- page HTML/markdown;
- passwords;
- request/response bodies.

### 27.2 Diagnostics

Lightpanda stderr is retained only in a bounded local diagnostics ring.

Before user-facing reporting:

- remove absolute Windows paths;
- remove WSL paths;
- remove stack traces unless explicitly requested in a developer diagnostic flow;
- redact token/password/secret-looking data.

### 27.3 Filesystem boundary

The managed WSL distro has no Windows drive automount and no Windows interop.

Browser page content therefore cannot use the browser runtime as a general bridge to Local Workspaces-approved files.

Workspace file access remains exclusively controlled by Local Workspaces filesystem tools.

---

## 28. Error model

Retain existing browser errors where applicable and add runtime-specific errors.

Required stable codes:

```text
BROWSER_AUTOMATION_DISABLED
BROWSER_NOT_RUNNING
BROWSER_START_FAILED
BROWSER_PROCESS_LOST
BROWSER_PROTOCOL_ERROR
BROWSER_STALE_REF
BROWSER_TARGET_NOT_FOUND
BROWSER_NAVIGATION_TIMEOUT
BROWSER_ACTION_TIMEOUT
BROWSER_FEATURE_UNSUPPORTED
BROWSER_ARTIFACT_FAILED

BROWSER_WSL_UNAVAILABLE
BROWSER_WSL_REBOOT_REQUIRED
BROWSER_WSL_DISTRO_CONFLICT
BROWSER_RUNTIME_NOT_READY
BROWSER_RUNTIME_DOWNLOAD_FAILED
BROWSER_RUNTIME_INTEGRITY_FAILED
BROWSER_RUNTIME_VERSION_MISMATCH
BROWSER_RUNTIME_PROVISION_FAILED
BROWSER_RUNTIME_REPAIR_REQUIRED
```

### Error sanitation

Never expose:

- command lines containing internal paths;
- upstream stack traces;
- raw WSL error dumps containing usernames or host paths;
- download temporary locations;
- cookies or credentials.

Desktop diagnostics may contain a sanitized technical detail code separate from MCP-facing messages.

---

## 29. Crash and recovery behavior

### Lightpanda child crash

When one page child dies unexpectedly:

1. mark that page lost;
2. clear its refs;
3. surface `BROWSER_PROCESS_LOST` for the in-flight call;
4. do not replay the call automatically;
5. keep other logical page children alive;
6. allow a subsequent explicit browser action to recreate a clean page when appropriate.

### WSL failure

If `wsl.exe` itself cannot start:

- return `BROWSER_WSL_UNAVAILABLE`;
- set runtime state to repair-required only when health probes confirm a persistent problem;
- do not unregister/rebuild the distro automatically on a single transient failure.

### App shutdown

App shutdown closes all Lightpanda children.

A bounded graceful shutdown may allow cookie jars to flush, followed by forced process-tree termination.

---

## 30. Removal of the Playwright runtime

The implementation must completely remove the release dependency on:

```text
Node.js browser runtime
@playwright/mcp
playwright
playwright-core
Playwright Chromium
Playwright Firefox
device-descriptors.json
PLAYWRIGHT_BROWSERS_PATH
```

The old `PlaywrightMcpDriver` is replaced by a Lightpanda-native driver.

No compatibility code is allowed to spawn Playwright after migration.

Old app-data browser profiles may be left untouched for one release so user data is not silently destroyed, but they are never used by the new runtime.

Desktop cleanup may later expose removal of legacy browser profile data.

---

## 31. Packaging and NSIS requirements

### 31.1 Tauri resources

Remove:

```text
browser-runtime/**/*
```

from release resources.

The release may include only small files such as:

```text
lightpanda-runtime-manifest.json
THIRD-PARTY-NOTICES
runtime license metadata
```

### 31.2 Build scripts

The current browser runtime build/verification scripts must be removed or replaced.

The new release pipeline verifies:

- no Node runtime is staged;
- no Playwright package is staged;
- no Chromium/Firefox browser folder is staged;
- runtime manifest has pinned versions/checksums;
- distro rootfs artifact metadata exists for release publishing.

### 31.3 Installer size gate

For Windows x86_64 v0.2.0:

```text
NSIS installer target: < 100 MiB
```

If the release exceeds 100 MiB, the release build fails until the size regression is understood.

The separately downloaded WSL runtime is not counted as NSIS size and must be reported separately in release notes.

### 31.4 Offline behavior

After WSL distro + Lightpanda provisioning completes successfully:

- browser processes must start without downloading runtime components;
- remote web navigation naturally still requires network access;
- runtime verification must not require an online version check.

---

## 32. Licensing and third-party notices

Lightpanda 0.3.7 is licensed under GNU AGPLv3.

Local Workspaces must:

- retain the Lightpanda license notice;
- identify the exact Lightpanda version used;
- provide the corresponding upstream source reference for that exact version;
- state whether the binary is unmodified;
- keep Lightpanda as a separate process/runtime component;
- include the runtime license notice in desktop third-party notices.

The managed Linux rootfs also needs an SBOM/package manifest and required distribution notices.

The release process must not publish a repackaged Lightpanda binary without satisfying the applicable redistribution obligations.

The preferred v0.2.0 flow downloads the unmodified pinned upstream binary and verifies its official digest.

---

## 33. Testing strategy

### 33.1 Unit tests

Required coverage:

- legacy settings -> Lightpanda migration;
- provisioning state transitions;
- WSL command argument construction;
- distro ownership verification;
- collision-safe distro naming;
- runtime manifest parsing;
- SHA-256 validation;
- runtime version mismatch;
- required MCP tool discovery;
- process-loss mapping;
- Pause priority;
- disabled priority;
- ref generation and stale refs;
- tree/backend-node -> opaque ref conversion;
- fixed-script string encoding;
- navigation follow-up verification;
- console ring cursors;
- performance/network normalization;
- unsupported tools fail before side effects;
- no model input enters shell/WSL command strings.

### 33.2 WSL integration tests

On a Windows WSL2 CI runner or dedicated test VM:

1. provision a fresh app-owned distro;
2. verify other installed distros are unchanged;
3. verify default distro is unchanged;
4. verify app ownership marker;
5. verify Windows drives are not automounted;
6. verify Windows interop is disabled;
7. verify Lightpanda runs as `localworkspaces`, not root;
8. verify exact Lightpanda version;
9. initialize native MCP over stdio;
10. verify required tools;
11. navigate a local deterministic fixture;
12. snapshot;
13. click;
14. fill;
15. select;
16. press;
17. hover;
18. wait;
19. evaluate;
20. console;
21. search in an online-enabled test;
22. markdown/html/links native wrappers;
23. find-element/interactive-elements native wrappers;
24. extract/structured-data/form-detection native wrappers;
25. scroll/set-checked native wrappers;
26. logical page new/select/close;
27. cookie persistence across browser restart;
28. SQLite storage persistence;
29. Pause termination;
30. Browser Automation OFF termination;
31. forced child crash -> process-lost error;
32. runtime repair after binary corruption;
33. runtime start with no visible console window.

### 33.3 Unsupported capability tests

Each must return exactly `BROWSER_FEATURE_UNSUPPORTED` and perform no hidden fallback:

```text
browser_screenshot
browser_set_viewport
browser_emulate_device
browser_dialog
browser_upload
browser_downloads
browser_save_download_to_workspace
```

`browser_devtools` returns support=false and launches nothing.

### 33.4 Installer tests

Release CI must:

- build NSIS;
- measure file size;
- assert `< 100 MiB`;
- inspect bundle resources;
- fail if any Node, Playwright, Chromium, or Firefox runtime is present.

### 33.5 Fresh-machine VM tests

Manual/release-gate test on a Windows VM without WSL:

1. install Local Workspaces;
2. enable/use Browser Automation;
3. verify UAC WSL install begins automatically;
4. verify no default Ubuntu distro is intentionally installed by Local Workspaces;
5. reboot when requested;
6. relaunch app;
7. verify provisioning resumes;
8. verify managed distro creation;
9. verify Lightpanda install;
10. verify first browser navigation succeeds.

This is a release gate because a normal unit test cannot faithfully simulate Windows optional-feature enablement and reboot.

---

## 34. Acceptance criteria

The feature is complete only when all of the following are true.

1. Release browser automation contains no Playwright MCP.
2. Release browser automation contains no Node.js runtime.
3. NSIS contains no Chromium or Firefox browser binaries.
4. Lightpanda is the only browser engine visible to users and agents.
5. Browser Settings has no browser selector.
6. Browser Settings has no Visible Debug mode.
7. A machine with working WSL2 can provision the app-owned distro without touching personal distros.
8. A machine without WSL can begin automatic WSL installation through the Windows-supported elevated flow.
9. Reboot-required state survives app restart and provisioning can resume.
10. The managed distro has a verifiable ownership marker.
11. Windows drive automount is disabled in the managed distro.
12. Windows interop is disabled in the managed distro.
13. Lightpanda runs as an unprivileged Linux user.
14. Lightpanda telemetry is disabled.
15. Lightpanda core dumps are disabled.
16. The installed Lightpanda version and SHA-256 match the pinned manifest.
17. MCP initialize + required tool discovery passes before runtime is marked ready.
18. `browser_open` works through Lightpanda native MCP.
19. Navigation is verified after `goto`, not trusted blindly.
20. Snapshot refs are opaque and stale-safe.
21. Core interaction tools work on deterministic fixtures.
22. The 10 native-first wrappers (`browser_markdown`, `browser_html`, `browser_links`, `browser_find_element`, `browser_interactive_elements`, `browser_extract`, `browser_structured_data`, `browser_detect_forms`, `browser_scroll`, `browser_set_checked`) work on deterministic fixtures.
23. Browser search uses Lightpanda native MCP.
24. Page JavaScript evaluation remains page-context-only.
25. Console data remains bounded and private.
26. Network/performance tools clearly report degraded non-CDP semantics.
27. Trace output is explicitly a Local Workspaces Lightpanda trace, not Playwright format.
28. Real screenshot calls fail explicitly and never return placeholder pixels.
29. No unsupported operation falls back to Chrome.
30. Pause immediately blocks new calls and terminates active Lightpanda processes.
31. Disabling Browser Automation terminates active Lightpanda processes.
32. Browser runtime can start offline after successful initial provisioning.
33. NSIS x86_64 artifact is below 100 MiB.
34. Full browser test suite passes on the pinned Lightpanda 0.3.7 runtime.
35. Existing unrelated uncommitted Local Workspaces work is not overwritten by the migration.

---

## 35. Explicit non-goals for v0.2.0

The following are intentionally out of scope:

- Chrome/Chromium fallback;
- Playwright or Puppeteer dependency;
- real screenshots;
- visual regression testing;
- visible browser windows;
- DevTools GUI;
- browser extensions;
- pixel-coordinate automation;
- full Chrome-equivalent device emulation;
- model-controlled WSL shell execution;
- exposing Lightpanda `agent` mode to ChatGPT;
- exposing arbitrary PandaScript execution;
- installing Lightpanda into user WSL distros;
- cloud Lightpanda fallback;
- automatic runtime updates independent of Local Workspaces releases.

A future version may adopt new native Lightpanda capabilities as they become stable, but it must preserve the no-silent-fallback rule unless the product decision is explicitly changed.

---

## 36. Expected codebase impact

The implementation plan should treat these as the primary responsibility boundaries.

### Rust browser backend

Replace the Playwright-specific driver with focused units for:

```text
browser/lightpanda.rs
    native MCP client/process adapter

browser/wsl.rs
    WSL host inspection, app-owned distro lifecycle

browser/runtime.rs
    provisioning state machine, manifest, integrity/update/repair

browser/manager.rs
    page lifecycle, logical tabs, refs, profile/cookie coordination

browser/mcp.rs
    stable Local Workspaces browser_* handlers and compatibility mapping
```

Exact file names may follow the repository's current module style, but responsibilities must stay separated.

### Settings/UI

Update:

```text
settings schema/migration
Tauri commands
Browser Automation Settings section
runtime status presentation
repair/reset/remove actions
```

### Build/release

Remove the existing Playwright browser runtime staging path and introduce:

```text
runtime manifest generation
minimal WSL rootfs release artifact
NSIS size gate
third-party notices/SBOM
```

### Tests

Replace Playwright-runtime integration fixtures with Lightpanda-native/Wsl fixtures while preserving the current Local Workspaces public `browser_*` contract tests.

---

## 37. Migration sequence

Implementation must land in an order that never produces a release containing two full browser stacks.

Recommended release sequence:

1. add runtime/settings migration types without changing release packaging;
2. add WSL/runtime provisioning behind tests;
3. add Lightpanda native MCP driver;
4. move core browser tools to Lightpanda;
5. implement adapters/degraded tools;
6. lock unsupported tool behavior;
7. switch BrowserManager default exclusively to Lightpanda;
8. remove UI engine/run-mode selectors;
9. remove Playwright/Node/browser runtime build scripts and resources;
10. run full compatibility tests;
11. build NSIS and enforce size gate;
12. bump app version to `0.2.0`;
13. produce release artifact.

At no point may a release silently choose Playwright because Lightpanda failed.

---

## 38. Success definition

The migration is successful when Local Workspaces can be installed as a small Windows desktop application, automatically establish its own isolated WSL2 browser environment when first needed, and provide agentic browser automation directly through Lightpanda's native MCP without Node, Playwright, Chromium, or Firefox.

The user-visible behavior should feel simpler than before:

- one Browser Automation toggle;
- one browser backend: Lightpanda;
- no browser/runtime selection;
- no heavyweight browser bundled in the installer;
- fast semantic automation;
- explicit capability boundaries instead of hidden fallbacks.

The architecture deliberately chooses machine-oriented browser semantics over graphical browser fidelity.
