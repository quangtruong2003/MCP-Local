# Lightpanda Native MCP + Managed WSL2 Implementation Plan

> **For agentic execution:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` task-by-task. Use `superpowers:test-driven-development` for every behavior change, `superpowers:systematic-debugging` for failures, and `superpowers:verification-before-completion` before claiming completion.

**Goal:** Replace bundled Playwright/Node/Chromium/Firefox with pinned Lightpanda `0.3.7` over native MCP private stdio inside an app-owned WSL2 distro; preserve the existing 29 browser tools, add 10 safe native-first tools (39 total), keep Browser Automation opt-in, and ship an NSIS under 100 MiB.

**Architecture:** Add a Windows WSL adapter (`browser/wsl.rs`), idempotent runtime provisioner (`browser/runtime.rs`), and Lightpanda page driver (`browser/lightpanda.rs`). Refactor `BrowserManager` from one Playwright process with upstream tabs to one Lightpanda child per logical page, with app-owned page IDs, ref generations, selector recovery, cookie snapshots, local trace/console state, and WSL-owned profile/cache/storage. Keep `WorkspaceServer` registration as thin audited wrappers while moving backend behavior into browser modules.

**Tech Stack:** Rust 2021, Tauri 2, Tokio, `rmcp` 3.1 child-process client, `reqwest`/rustls, Windows `wsl.exe`, Win32 `ShellExecuteExW` elevation, WSL2 Ubuntu 24.04 LTS minbase, Lightpanda `0.3.7`, React 18/TypeScript/Vite, PowerShell release verification, GitHub Actions.

---

## Review decisions already incorporated

The spec `docs/superpowers/specs/2026-08-27-lightpanda-native-mcp-wsl-design.md` was reviewed against the current repo and the pinned upstream Lightpanda contract. The spec was corrected in-place for implementation-critical issues:

1. Ref-based actions use `Local ref -> backendNodeId -> nodeDetails.selector -> action`; they do not pass `backendNodeId` straight to click/fill/hover/select/check.
2. `browser_wait.load_state` preserves public values `loading | interactive | complete`; Lightpanda-native names stay internal.
3. `--cookie` and `--cookie-jar` are file-path arguments.
4. Destructive WSL ownership requires host-generated `install_id` plus the expected managed install location.
5. Ubuntu rootfs URL/SHA are generated only after the release artifact exists; no mutable/unpublished URL is hard-coded.

Repository constraints:

- `src-tauri/tests/browser_mcp_integration.rs` currently pins exactly 29 public browser tools.
- Browser handlers are concentrated in `src-tauri/src/server.rs`; keep tool macros/wrappers there initially and move logic below them.
- `BrowserDriver`, manager, settings UI, and the real runtime integration suite are Playwright-shaped and require explicit replacement.
- The current working tree has uncommitted browser work. **Never use `git reset`, `git checkout --`, destructive clean commands, or overwrite it.**

## Execution preflight

Record baseline without changing it:

```powershell
git status --short --branch
git diff --check
git diff --name-only
```

Run baseline tests before Task 1:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration
npm test
npm run build
```

Do not run old `npm run build:browser-runtime` as a migration prerequisite.

---

## Task 1: Fixed Lightpanda settings model + runtime error codes

**Files:**
- Modify: `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/browser/types.rs`
- Modify: `crates/core/src/error.rs`
- Modify: `src/lib/types.ts`
- Test: inline Rust unit tests in settings/error modules

### Step 1: Write failing migration tests

Add v5 fixtures for all legacy engines and both run modes. Assert load -> version 6, `browser_automation_enabled` preserved, and save -> no serialized `browser_engine`/`browser_run_mode`.

```rust
#[test]
fn v5_browser_choices_migrate_to_v6_fixed_lightpanda_backend() {
    // write version:5, browser_engine:"firefox", browser_run_mode:"visible_debug"
    // load -> version 6; save -> legacy choice fields absent
}
```

Also assert defaults are schema version 6.

### Step 2: Verify RED

```powershell
cargo test --manifest-path src-tauri/Cargo.toml settings::tests::v5_browser_choices_migrate_to_v6_fixed_lightpanda_backend -- --exact
```

### Step 3: Add failing stable-code assertions

Extend `error_codes_are_stable_strings` for:

```text
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

Run the error test and confirm RED.

### Step 4: Implement minimal migration

- bump settings to v6;
- remove user-owned engine/run-mode fields from current serialized `AppSettings`;
- keep only `browser_automation_enabled`;
- add runtime-only status identities in browser types: `BrowserBackend::Lightpanda`, `BrowserRunMode::Background`;
- simplify `BrowserRuntimeSettings` to `{ enabled }`;
- remove engine/run-mode fields from TypeScript `AppSettings`;
- add the nine error variants/mappings without changing existing codes.

### Step 5: Verify GREEN

```powershell
cargo test --manifest-path src-tauri/Cargo.toml settings::tests
cargo test --manifest-path src-tauri/Cargo.toml -p core error::tests
npm run build
```

### Step 6: Commit

```powershell
git add src-tauri/src/settings.rs src-tauri/src/browser/types.rs crates/core/src/error.rs src/lib/types.ts
git commit -m "refactor(browser): fix runtime settings to lightpanda"
```

---

## Task 2: Build/publish Ubuntu minbase rootfs and generate immutable runtime manifest

**Files:**
- Create: `scripts/build-lightpanda-rootfs.sh`
- Create: `scripts/generate-lightpanda-runtime-manifest.ps1`
- Create: `scripts/verify-lightpanda-runtime-manifest.ps1`
- Create: `.github/workflows/lightpanda-runtime.yml`
- Create/generated: `src-tauri/resources/lightpanda-runtime-manifest.json`
- Create/generated: `third-party/lightpanda-rootfs-packages.txt`
- Create/generated: `third-party/lightpanda-rootfs-sbom.spdx.json`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `package.json`

### Step 1: Write verifier first and confirm RED

Verifier requires:

```json
{
  "schema_version": 1,
  "runtime_family": "lightpanda-wsl",
  "lightpanda_version": "0.3.7",
  "lightpanda_arch": "x86_64-linux",
  "lightpanda_asset_url": "https://github.com/lightpanda-io/browser/releases/download/0.3.7/lightpanda-x86_64-linux",
  "lightpanda_sha256": "895339b02205171a181dde743ae0068bb4564884076feac8482baca9c212aa5a",
  "distro_image_version": "lwsl-ubuntu-noble-minbase-v1",
  "distro_arch": "amd64",
  "distro_image_url": "<immutable published URL>",
  "distro_image_sha256": "<64 lowercase hex>"
}
```

Reject placeholders, `nightly`, mutable rootfs URL, wrong digest/arch, or missing package-list/SBOM.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-lightpanda-runtime-manifest.ps1
```

Expected RED before artifact/manifest exists.

### Step 2: Implement deterministic rootfs build

On Ubuntu 24.04 CI:

1. install `debootstrap`, tar/curl/CA/SPDX tooling;
2. `debootstrap --variant=minbase --arch=amd64 noble` from official Ubuntu archive;
3. install only DNS/TLS/Lightpanda runtime libs; begin with `ca-certificates libc6 libgcc-s1 libstdc++6`, verify with `ldd` + smoke launch, remove unnecessary packages;
4. create required `/var/lib/local-workspaces/browser/*` dirs but no profile/cookie data;
5. assert Node/npm/Playwright/Chromium/Firefox/toolchains are absent;
6. emit dpkg manifest + SPDX SBOM;
7. create deterministic tar (sorted entries, fixed owner/group, normalized mtime) suitable for `wsl.exe --import`;
8. output SHA-256.

Do not bake Lightpanda into the rootfs; app provisions the pinned upstream binary separately.

### Step 3: Publish artifact before app release

Workflow builds on `ubuntu-24.04`, uploads tar/package list/SBOM, and attaches them to an immutable runtime tag such as `runtime-lwsl-ubuntu-noble-minbase-v1`. No app release may consume a floating `latest` rootfs.

### Step 4: Generate manifest from published URL/SHA

`generate-lightpanda-runtime-manifest.ps1` accepts explicit rootfs URL + SHA and writes the manifest atomically; Lightpanda URL/SHA are fixed to reviewed 0.3.7 values.

### Step 5: Bundle only small manifest/notices

Add only the generated manifest/notices to Tauri resources. Add npm command `verify:lightpanda-runtime`. Do not delete the old runtime yet; Task 15 removes it after new runtime tests are green.

### Step 6: Verify GREEN and commit

```powershell
npm run verify:lightpanda-runtime
git add scripts .github/workflows/lightpanda-runtime.yml src-tauri/resources third-party package.json src-tauri/tauri.conf.json
git commit -m "build(browser): add pinned lightpanda runtime manifest"
```

---

## Task 3: Testable Windows WSL host adapter + collision-safe ownership

**Files:**
- Create: `src-tauri/src/browser/wsl.rs`
- Modify: `src-tauri/src/browser/mod.rs`
- Modify: `src-tauri/Cargo.toml`
- Test: `src-tauri/src/browser/wsl.rs`

### Step 1: Add runner seam and parsing tests

```rust
#[async_trait]
pub trait WslCommandRunner: Send + Sync {
    async fn run(&self, request: WslCommand) -> Result<WslCommandOutput, CoreError>;
    fn launch_elevated_install(&self) -> Result<(), CoreError>;
}
```

Tests cover `wsl --status`, `--version`, `--list --verbose`, WSL absent, WSL1 entries, non-zero exit, and sanitized public errors.

### Step 2: Add ownership/collision tests

Persist:

```rust
struct ManagedDistroIdentity {
    distro_name: String,
    install_id: String,
    install_dir: PathBuf,
}
```

Assert:

- free preferred name -> `LocalWorkspaces-Lightpanda`;
- collision not owned -> suffix with generated 8-char install ID;
- reuse only when marker owner/runtime/install_id **and** managed install location all match;
- unregister/repair is refused on any ownership mismatch.

### Step 3: Verify RED

```powershell
cargo test --manifest-path src-tauri/Cargo.toml browser::wsl::tests -- --nocapture
```

### Step 4: Implement fixed operations only

Expose fixed methods: host inspection, distro list/import, marker read/write, provisioning commands, Lightpanda child launch, owned-distro terminate/unregister. Never accept arbitrary model-controlled shell commands.

Use exact import shape:

```text
wsl.exe --import <name> <managed-install-dir> <rootfs-tar> --version 2
```

Add Windows `Win32_UI_Shell` feature and use `runas`/`ShellExecuteExW` for `wsl.exe --install --no-distribution`; never bypass UAC.

### Step 5: Verify GREEN and commit

```powershell
cargo test --manifest-path src-tauri/Cargo.toml browser::wsl::tests
git add src-tauri/src/browser/wsl.rs src-tauri/src/browser/mod.rs src-tauri/Cargo.toml
git commit -m "feat(browser): add managed wsl host adapter"
```

---

## Task 4: Idempotent WSL/Lightpanda runtime provisioner

**Files:**
- Create: `src-tauri/src/browser/runtime.rs`
- Modify: `src-tauri/src/browser/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/browser/runtime.rs`
- Modify/Test: `src-tauri/tests/browser_runtime_integration.rs`

### Step 1: Write state-machine tests first

Use exact states from spec:

```text
uninitialized checking_host wsl_install_required installing_wsl reboot_required
distro_install_required installing_distro lightpanda_install_required installing_lightpanda
verifying_runtime ready repair_required unsupported_host
```

Test healthy reuse, absent distro provisioning, WSL install + reboot persistence/resume, checksum failure before import, Lightpanda checksum failure before symlink switch, interrupted `.partial` recovery, concurrent `ensure_runtime()` serialization, and transient WSL failure with no destructive repair.

### Step 2: Verify RED

```powershell
cargo test --manifest-path src-tauri/Cargo.toml browser::runtime::tests -- --nocapture
```

### Step 3: Implement manifest/download/integrity layer

- typed immutable manifest;
- HTTPS-only runtime downloads (except cfg(test) localhost fixture);
- bounded redirects/timeouts;
- `.partial` + atomic rename;
- SHA-256 verification before import/install;
- sanitized errors without host paths/cookies.

### Step 4: Provision owned distro

After import, fixed root commands:

- write `/etc/wsl.conf` with automount/interop disabled;
- create `localworkspaces` user/home;
- create/chown browser dirs;
- write marker with host `install_id`;
- install root-owned `/opt/local-workspaces/lightpanda/0.3.7/lightpanda`;
- verify digest/version before switching `current` symlink;
- create root-owned launcher that sets telemetry/core-dump controls and executes native MCP as unprivileged user;
- terminate only the owned distro after changing `wsl.conf`, then re-verify.

### Step 5: Verify runtime before `Ready`

Require WSL2 health, owned distro identity/version, isolation config, exact Lightpanda digest/version, unprivileged launch, and native MCP initialize/tool discovery. Only then set `ready`.

Persist only small atomic `runtime-state.json` with state + distro identity/install ID + manifest version + sanitized reason codes.

### Step 6: Wire typed runtime service into AppState

Replace Playwright runtime-path diagnostics field with `Arc<BrowserRuntimeManager>`. Provisioning remains lazy; opening Settings alone must not download/install.

### Step 7: Verify GREEN and commit

```powershell
cargo test --manifest-path src-tauri/Cargo.toml browser::runtime::tests
cargo test --manifest-path src-tauri/Cargo.toml --lib
git add src-tauri/src/browser/runtime.rs src-tauri/src/browser/mod.rs src-tauri/src/lib.rs src-tauri/tests/browser_runtime_integration.rs
git commit -m "feat(browser): add lightpanda runtime provisioner"
```

## Task 5: Native Lightpanda MCP page driver

**Files:**
- Modify: `src-tauri/src/browser/driver.rs`
- Create: `src-tauri/src/browser/lightpanda.rs`
- Modify: `src-tauri/src/browser/mod.rs`
- Create/Test fixture: `src-tauri/tests/fixtures/fake_lightpanda_mcp.rs`
- Modify: `src-tauri/Cargo.toml`

### Step 1: Redesign driver around one page

Replace Playwright-specific methods (CDP metrics, device descriptors, DevTools launch checks) with:

```rust
#[async_trait]
pub trait BrowserPageDriver: Send + Sync {
    async fn start(&self, spec: PageLaunchSpec) -> Result<PageDriverStatus, CoreError>;
    async fn call(&self, tool: &'static str, args: Map<String, Value>) -> Result<CallToolResult, CoreError>;
    async fn stop(&self) -> Result<(), CoreError>;
    async fn is_alive(&self) -> bool;
}

pub trait BrowserPageDriverFactory: Send + Sync {
    fn create(&self) -> Arc<dyn BrowserPageDriver>;
}
```

`PageLaunchSpec` contains only generated page ID and fixed WSL-owned profile/storage/cache/cookie paths.

### Step 2: Write fake-child protocol tests first

Fake native MCP child modes: normal initialize/discovery, missing required tool, extra `save/session_*`, malformed JSON, clean EOF/crash, delayed call, and sensitive-looking stderr for sanitation tests.

Pin required upstream discovery:

```text
goto search markdown html links evaluate extract tree nodeDetails
interactiveElements structuredData detectForms click fill scroll
waitForSelector waitForScript waitForState hover press selectOption
setChecked findElement consoleLogs getUrl getCookies getEnv
```

`save`, `session_*`, `getEnv`, and `getCookies` are never directly public Local Workspaces tools; only internal code may call `getCookies`.

### Step 3: Verify RED

```powershell
cargo test --manifest-path src-tauri/Cargo.toml browser::lightpanda::tests -- --nocapture
```

### Step 4: Implement private stdio launch through WSL

Use `rmcp` child-process transport and the app-owned launcher as `localworkspaces`. App-generated args only:

```text
mcp
--cookie <canonical-cookie-json-path>
--cookie-jar <page-specific-exit-json-path>
--storage-engine sqlite
--storage-sqlite-path /var/lib/local-workspaces/browser/profile/storage.sqlite
--http-cache-dir /var/lib/local-workspaces/browser/cache
--http-cache-entry-limit <bounded constant>
--log-level warn
```

Launcher sets `LIGHTPANDA_DISABLE_TELEMETRY=true`, `LIGHTPANDA_DISABLE_CORE_DUMP=1`, `ulimit -c 0`, then `exec`s Lightpanda. Never use Lightpanda HTTP MCP transport.

Map child loss -> `BROWSER_PROCESS_LOST`, startup/discovery -> stable start/runtime errors, malformed protocol -> `BROWSER_PROTOCOL_ERROR`, and timeouts -> existing timeout codes. Public errors must not contain cookie paths/command lines/raw stderr.

### Step 5: Verify GREEN and commit

```powershell
cargo test --manifest-path src-tauri/Cargo.toml browser::lightpanda::tests
git add src-tauri/src/browser/driver.rs src-tauri/src/browser/lightpanda.rs src-tauri/src/browser/mod.rs src-tauri/tests/fixtures/fake_lightpanda_mcp.rs src-tauri/Cargo.toml
git commit -m "feat(browser): add native lightpanda mcp page driver"
```

---

## Task 6: BrowserManager one-child-per-page + WSL profile/cookies

**Files:**
- Modify: `src-tauri/src/browser/manager.rs`
- Modify: `src-tauri/src/browser/types.rs`
- Modify: `src-tauri/src/browser/driver.rs`
- Modify/Test: `src-tauri/tests/browser_mcp_integration.rs`

### Step 1: Convert fake harness to a driver factory

Write failing tests proving open creates one child, `tabs new` creates another, select does not restart, close stops only one, one crash leaves sibling alive, and Pause/disable/shutdown stop all children. Assert no upstream `browser_tabs` primitive exists.

### Step 2: Introduce manager-owned state

```rust
struct ManagedPage {
    page_id: String,
    driver: Arc<dyn BrowserPageDriver>,
    generation: u64,
    refs: HashMap<String, BackendNodeRef>,
    url: Option<String>,
    title: Option<String>,
    lost: bool,
    console: ConsoleRing,
    trace: Option<TraceState>,
    cookie_jar_path: String,
}

struct ActiveSession {
    session_id: String,
    pages: Vec<ManagedPage>,
    active_page_id: String,
}
```

Keep generated IDs opaque and never return WSL paths.

### Step 3: Enforce guard order

```text
Pause -> Browser Automation enabled -> runtime ready -> session/page -> request validation -> action
```

Status may inspect runtime without causing setup downloads; existing Pause policy remains authoritative.

### Step 4: Use shared WSL-owned state

All page children share:

```text
/var/lib/local-workspaces/browser/profile/storage.sqlite
/var/lib/local-workspaces/browser/cookies/
/var/lib/local-workspaces/browser/cache/
```

Remove Windows browser profile logic from active manager path.

### Step 5: Canonical cookie snapshot + page exit jars

Implement internal helpers to refresh canonical cookies with `getCookies(all=true)`, write canonical JSON through fixed WSL file operations, launch each page with canonical cookie path + unique exit jar, merge after state-changing interactions/orderly shutdown, and never audit/return cookie values.

Add a test for the documented limitation: already-running sibling pages do not receive live cookie updates; newly launched pages do.

### Step 6: Shared SQLite concurrency test scaffold

Start two page children against the same SQLite storage and perform concurrent storage reads/writes. This test is mandatory under the release runtime flag later.

### Step 7: Verify and commit

```powershell
cargo test --manifest-path src-tauri/Cargo.toml browser::manager::tests
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration lifecycle_tabs -- --nocapture
git add src-tauri/src/browser/manager.rs src-tauri/src/browser/types.rs src-tauri/src/browser/driver.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "refactor(browser): manage one lightpanda child per page"
```

---

## Task 7: Opaque ref generations + selector recovery

**Files:**
- Modify: `src-tauri/src/browser/manager.rs`
- Modify: `src-tauri/src/browser/mcp.rs`
- Modify/Test: `src-tauri/tests/browser_mcp_integration.rs`

### Step 1: Write failing ref tests

Cover:

- native backend node IDs become `e1/e2/...`;
- refs are scoped to `(page_id, generation)`;
- navigation/reload/restart/major mutation invalidates refs;
- stale/cross-page refs fail before native action;
- click/fill/hover/select/setChecked perform `nodeDetails` then use returned CSS selector;
- public `eN`/backend IDs never reach action arguments;
- selectors inserted into fixed JS are JSON-encoded data.

### Step 2: Verify RED

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration ref_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration interaction_ -- --nocapture
```

### Step 3: Implement page-scoped ref table

```rust
fn register_backend_node(page_id, generation, backend_node_id) -> String;
fn resolve_ref(page_id, ref_id) -> Result<BackendNodeRef, CoreError>;
async fn selector_for_ref(page_id, ref_id) -> Result<String, CoreError>;
fn invalidate_refs(page_id);
```

Normalize `tree`, `findElement`, `interactiveElements`, and `links` so returned backend node IDs become opaque refs; bound outputs and never persist refs across restart.

### Step 4: Verify GREEN and commit

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration ref_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration interaction_ -- --nocapture
git add src-tauri/src/browser/manager.rs src-tauri/src/browser/mcp.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "feat(browser): add lightpanda opaque ref adapter"
```

---

## Task 8: Expand discovery from 29 to 39 Local Workspaces browser tools

**Files:**
- Modify: `src-tauri/src/browser/mcp.rs`
- Modify: `src-tauri/src/server.rs`
- Modify/Test: `src-tauri/tests/browser_mcp_integration.rs`

### Step 1: Change discovery test first

Require exactly 39 tools with `outputSchema`, adding:

```text
browser_markdown
browser_html
browser_links
browser_find_element
browser_interactive_elements
browser_extract
browser_structured_data
browser_detect_forms
browser_scroll
browser_set_checked
```

Also assert raw upstream `getEnv`, `getCookies`, `save`, `session_new`, `session_list`, `session_close` are absent.

### Step 2: Verify RED

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration browser_discovery_exposes_exactly_39_public_browser_tools -- --exact --nocapture
```

### Step 3: Add typed args/output schemas

In `browser/mcp.rs` add validation for bounded `max_bytes`/timeouts, find-element role/name requirement, extract encoded size/depth/selector-count/limit, scroll bounds, and set-checked default `true`.

Keep model-facing types separate from raw upstream maps.

### Step 4: Register thin audited wrappers

Keep tool macros in `server.rs`; wrappers delegate to browser modules and audit only safe counts/bytes/opaque targets. Do not expose a generic native-tool call surface.

### Step 5: Verify GREEN and commit

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration browser_discovery_exposes_exactly_39_public_browser_tools -- --exact --nocapture
git add src-tauri/src/browser/mcp.rs src-tauri/src/server.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "feat(browser): expose 39-tool lightpanda contract"
```

---

## Task 9: Port lifecycle, navigation, search, snapshot, actions, evaluate, waits

**Files:**
- Modify: `src-tauri/src/browser/manager.rs`
- Modify: `src-tauri/src/browser/mcp.rs`
- Modify: `src-tauri/src/server.rs`
- Modify/Test: `src-tauri/tests/browser_mcp_integration.rs`

### Step 1: Rewrite fake expectations to native mappings

```text
browser_open(url)     -> ensure runtime -> child -> goto -> getUrl + bounded tree/markdown verification
browser_navigate(url) -> goto -> getUrl + verification
browser_search        -> search
browser_snapshot      -> tree -> ref normalization
browser_click         -> nodeDetails -> click(selector)
browser_hover         -> nodeDetails -> hover(selector)
browser_fill          -> nodeDetails -> fill(selector,value)
browser_select        -> nodeDetails -> selectOption(selector,...)
browser_press         -> press(key)
browser_evaluate      -> evaluate(page script)
```

Back/forward/reload use fixed application-owned evaluate scripts and invalidate refs.

### Step 2: Navigation correctness tests

Test URL policy, `goto` success followed by failed verification -> navigation error, final `getUrl`, origin-only audit, query-byte-only search audit, and assert no Google URL synthesis remains.

### Step 3: Preserve existing wait schema

Write tests for `text_present`, `text_absent`, `ref_visible`, `ref_hidden`, `url_contains`, `load_state`, and `delay`.

Mappings:

```text
text/url/ref-hidden -> fixed waitForScript with encoded data
ref-visible         -> selector recovery + waitForSelector/bounded DOM-CSS predicate
loading             -> waitForScript(document.readyState === "loading")
interactive         -> waitForScript(document.readyState === "interactive")
complete            -> waitForState("load") + readyState verification if needed
delay               -> host sleep in Pause-interruptible chunks
```

Never replace public load states with `load/domcontentloaded/networkidle`.

### Step 4: RED -> minimal implementation -> GREEN

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration navigation_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration interaction_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration wait_ -- --nocapture
```

On child loss, return `BROWSER_PROCESS_LOST`, mark only that page lost, and never replay state-changing action. Add a side-effect counter test.

### Step 5: Commit

```powershell
git add src-tauri/src/browser/manager.rs src-tauri/src/browser/mcp.rs src-tauri/src/server.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "feat(browser): port core tools to lightpanda native mcp"
```

## Task 10: Implement all ten native-first wrappers

**Files:** `src-tauri/src/browser/manager.rs`, `src-tauri/src/browser/mcp.rs`, `src-tauri/src/server.rs`, `src-tauri/tests/browser_mcp_integration.rs`.

### Step 1: Add one failing test per tool

Assert exact mapping/bounds:

- `browser_markdown` -> `markdown`, optional validated URL/ref scope, bounded bytes;
- `browser_html` -> `html`, optional ref scope, bounded bytes;
- `browser_links` -> `links`, normalize text/href/ref;
- `browser_find_element` -> `findElement`, normalize refs;
- `browser_interactive_elements` -> `interactiveElements`, bounded/ref-normalized;
- `browser_extract` -> validate object, serialize once into upstream schema string, parse structured result;
- `browser_structured_data` -> `structuredData`;
- `browser_detect_forms` -> `detectForms`, never expose password values;
- `browser_scroll` -> `scroll`; with ref use pinned upstream locator form (prefer recovered selector where supported), otherwise page scroll;
- `browser_set_checked` -> selector recovery -> `setChecked`.

For content tools, test truncation metadata and audit byte/count-only behavior.

### Step 2: Verify RED

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration native_first_ -- --nocapture
```

### Step 3: Implement through browser adapter helpers

`server.rs` must not construct raw native maps. Centralize upstream names/argument shapes in browser modules.

For `browser_extract`, reject before native call: non-object root, excessive depth, selector count, encoded bytes, negative/excessive limits, unsupported shapes. Serialize strings as data; never concatenate them into executable JS.

### Step 4: Verify GREEN and commit

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration native_first_ -- --nocapture
git add src-tauri/src/browser/manager.rs src-tauri/src/browser/mcp.rs src-tauri/src/server.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "feat(browser): add lightpanda native-first tools"
```

---

## Task 11: Truthful DOM/console/network/performance/drag/trace adapters

**Files:** `src-tauri/src/browser/mcp.rs`, `src-tauri/src/browser/manager.rs`, `src-tauri/src/browser/artifacts.rs`, `src-tauri/src/server.rs`, `src-tauri/tests/browser_mcp_integration.rs`.

### Step 1: Write failing compatibility tests

`browser_dom`: `nodeDetails` + ref-scoped `html` + fixed computed-style allowlist; redact sensitive values; never fabricate layout rect (legacy rect is `null`/unsupported if kept for shape compatibility).

`browser_console`: `consoleLogs` -> bounded app ring/cursor; preserve limit/level/cursor; never audit message text.

`browser_network`: fixed page evaluate over Performance Resource Timing; no fake bodies/headers/CDP; explicit degraded/coverage marker; keep URL/header redaction.

`browser_performance`: Navigation/Resource Performance APIs only; backend `lightpanda`; no CDP/renderer metrics or fake `cdp_supported=true`.

`browser_drag`: recover start/end selectors; fixed synthetic HTML5 `DataTransfer` sequence; no coordinate API; unsupported if real rendered coordinates are required.

`browser_trace`: app-owned JSONL/manifest; opaque artifact IDs; no Playwright trace zip; exclude cookies, fill text, evaluate source, page content and absolute paths.

### Step 2: Verify RED

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration dom_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration console_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration network_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration performance_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration trace_ -- --nocapture
```

### Step 3: Implement and remove active CDP assumptions

Delete active manager/server calls to Chromium metrics and Playwright trace primitives as tests turn green. Keep old source only until Task 15 removes it after real Lightpanda validation.

### Step 4: Verify GREEN + full suite and commit

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration
git add src-tauri/src/browser/mcp.rs src-tauri/src/browser/manager.rs src-tauri/src/browser/artifacts.rs src-tauri/src/server.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "refactor(browser): add truthful lightpanda compatibility adapters"
```

## Task 12: Unsupported legacy tools fail closed + run_steps full preflight

**Files:** `src-tauri/src/server.rs`, `src-tauri/src/browser/mcp.rs`, `src-tauri/src/browser/files.rs`, `src-tauri/tests/browser_mcp_integration.rs`.

### Step 1: Add side-effect sentinel tests

These remain discoverable but return `BROWSER_FEATURE_UNSUPPORTED` (DevTools may use structured `supported=false`) before filesystem/native/session side effects:

```text
browser_set_viewport
browser_emulate_device
browser_screenshot
browser_dialog
browser_upload
browser_downloads
browser_save_download_to_workspace
browser_devtools
```

For upload assert no staging file/dir. For save-download assert destination untouched. For screenshot/viewport/emulation/dialog/devtools assert zero page-driver calls.

Pause and Browser Automation checks stay higher priority; unsupported is returned immediately afterward.

### Step 2: Verify RED

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration unsupported_ -- --nocapture
```

### Step 3: Move unsupported checks before request-specific side effects

Do not resolve workspace files, create artifacts, change session state, or call native MCP before unsupported return.

### Step 4: Preflight every `browser_run_steps` item before step 0

Validate all steps for allowed type, required fields, URL validity, ref/key/text/value bounds, delay/timeout bounds, and unsupported types. Add a test with side-effecting step 0 + invalid step 1; assert step 0 never ran. Preserve stop-on-first-runtime-failure and no replay.

### Step 5: Verify GREEN and commit

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration unsupported_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration run_steps_ -- --nocapture
git add src-tauri/src/server.rs src-tauri/src/browser/mcp.rs src-tauri/src/browser/files.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "fix(browser): fail closed for unsupported lightpanda features"
```

---

## Task 13: Replace desktop diagnostics/settings with WSL Lightpanda controls

**Files:** `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`, `src/api.ts`, `src/lib/types.ts`, `src/pages/settings-page.tsx`.

### Step 1: Define redacted runtime status test-first

Status includes only: enabled/running, backend `lightpanda`, mode `background`, runtime state, Lightpanda version, WSL/distro/reboot/repair booleans, and truthful capability flags (`visual_rendering=false`, `screenshots=false`, `file_upload=false`, `managed_downloads=false`, `native_mcp=true`).

Rust test must prove serialization excludes URL/title/history/cookie/absolute host paths/download filenames.

### Step 2: Add desktop-only admin commands

```text
browser_runtime_status
setup_browser_runtime
repair_browser_runtime
reset_browser_data
remove_browser_runtime
```

Rules:
- setup/repair call provisioner and return fresh redacted status;
- reset closes children then clears only owned WSL profile/cookies/cache;
- remove closes children then unregisters/deletes only after full ownership check;
- MCP browser tools never expose runtime administration;
- UI audit uses logical action/IDs, never absolute runtime paths.

### Step 3: Rewrite Browser Automation settings card

Remove engine selector, Visible Debug selector, Node/Playwright versions, engine-specific profile reset. Show toggle, fixed `Lightpanda (WSL2)` + `Background`, runtime state/version, Setup/Resume/Repair, reboot guidance, capability limitations, Reset browser data and Remove browser runtime confirmations.

Opening Settings alone must not start downloads/provisioning.

### Step 4: Verify and commit

```powershell
cargo test --manifest-path src-tauri/Cargo.toml commands::tests
npm test
npm run build
git add src-tauri/src/commands.rs src-tauri/src/lib.rs src/api.ts src/lib/types.ts src/pages/settings-page.tsx
git commit -m "feat(browser): add lightpanda wsl runtime settings ux"
```

---

## Task 14: Rewrite real runtime integration from Playwright to managed WSL2 Lightpanda

**Files:**
- Rewrite: `src-tauri/tests/browser_runtime_integration.rs`
- Modify: `src-tauri/tests/fixtures/browser_test_app.rs`
- Modify/Create: `src-tauri/tests/fixtures/fake_lightpanda_mcp.rs`

### Step 1: Remove Playwright-only assumptions

Delete runtime tests/helpers for Chromium/Firefox/Chrome/Edge matrix, Visible Debug/DevTools, screenshots, Playwright upload/download artifacts, CDP metrics, Playwright trace zip, and staged Node/browser-runtime manifest.

### Step 2: Add mandatory real Lightpanda scenarios

Using deterministic localhost fixture, cover:

1. already-provisioned runtime -> ready;
2. Background launch with no visible browser UI;
3. navigation + verified final URL;
4. tree -> opaque ref -> selector-resolved fill/select/click;
5. all ten native-first tools;
6. page-context evaluate;
7. every existing wait condition/load-state value;
8. console ring;
9. degraded network/performance shape;
10. local trace artifact;
11. two page children and isolated crash;
12. shared SQLite concurrent access;
13. cookie persistence and documented non-live sibling behavior;
14. stale refs after navigation/reload;
15. crash/no-replay side-effect counter;
16. unsupported tools have zero side effects;
17. Pause and disable terminate all page children;
18. reset only owned browser data;
19. remove refuses ownership marker/install-ID mismatch.

### Step 3: Make release skips impossible

Developer runs may skip real WSL tests only when prerequisites are unavailable and must print a reason. When `LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS=1`, any skip is a test failure.

### Step 4: Run real suite on Windows/WSL2

```powershell
$env:LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS='1'
cargo test --manifest-path src-tauri/Cargo.toml --test browser_runtime_integration --jobs 1 -- --nocapture
Remove-Item Env:LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS
```

Expected: zero skips.

### Step 5: Commit

```powershell
git add src-tauri/tests/browser_runtime_integration.rs src-tauri/tests/fixtures/browser_test_app.rs src-tauri/tests/fixtures/fake_lightpanda_mcp.rs
git commit -m "test(browser): cover real lightpanda wsl runtime"
```

## Task 15: Delete Playwright runtime only after real Lightpanda suite is green

**Files:**
- Delete: `src-tauri/src/browser/playwright.rs`
- Delete: `scripts/build-browser-runtime.ps1`
- Delete: `scripts/verify-browser-runtime.ps1`
- Remove project runtime directory: `src-tauri/browser-runtime/` if present
- Modify: `src-tauri/src/browser/mod.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/commands.rs`, `src-tauri/Cargo.toml`, `package.json`, lockfiles, `src-tauri/tauri.conf.json`

### Step 1: Add legacy-runtime release scan and confirm RED

Fail active source/build/package references to:

```text
@playwright/mcp
playwright-core
PLAYWRIGHT_BROWSERS_PATH
browser-runtime
PlaywrightMcpDriver
node.exe
device-descriptors.json
```

Historical docs may be excluded by path; active release source/config may not.

### Step 2: Remove old dependencies/resources/code

- remove `@playwright/mcp` from package devDependencies;
- remove old browser runtime npm scripts;
- remove `browser-runtime/**/*` from Tauri resources;
- remove Playwright driver/module/construction and CDP/device/DevTools-only driver methods;
- regenerate lockfiles normally;
- remove `image`/`base64` Rust dependencies only if repo search proves screen-capture or other features do not use them.

### Step 3: Verify scan and tests

```powershell
npm install
npm run verify:lightpanda-runtime
git grep -n -E "@playwright/mcp|PLAYWRIGHT_BROWSERS_PATH|browser-runtime|PlaywrightMcpDriver" -- ':!docs/superpowers/**'
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration
npm test
npm run build
```

Expected git grep: no active source/build match.

### Step 4: Commit

```powershell
git add -A
git commit -m "build(browser): remove bundled playwright runtime"
```

---

## Task 16: Licensing/SBOM/source-reference + NSIS release gates

**Files:**
- Create/Modify: `THIRD_PARTY_NOTICES.md`
- Create: `third-party/lightpanda-source-reference.txt`
- Create: `scripts/verify-browser-release.ps1`
- Modify: `package.json`
- Modify: `.github/workflows/lightpanda-runtime.yml`
- Create: `.github/workflows/windows-release.yml` if no release workflow exists

### Step 1: Write release verifier first and confirm RED

Require all:

- exact Lightpanda 0.3.7 URL/SHA from manifest;
- AGPL/source reference present;
- rootfs package manifest/SBOM/notices present;
- no Playwright/Node/browser engines bundled;
- immutable rootfs URL/SHA present;
- NSIS exists and is `< 100 MiB`;
- release runtime integration log indicates zero skips.

### Step 2: Add notices/source reference

Document Lightpanda repo/tag/source URL, exact unmodified x86_64 binary digest, AGPLv3 obligations/source availability, Ubuntu 24.04 rootfs package/SBOM references, and that runtime artifacts are downloaded separately rather than included in NSIS.

### Step 3: Add Windows release job

Run, in order:

```text
npm ci
npm run verify:lightpanda-runtime
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration
LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS=1 cargo test --manifest-path src-tauri/Cargo.toml --test browser_runtime_integration --jobs 1 -- --nocapture
npm run build
npm run tauri build
powershell ... scripts/verify-browser-release.ps1
```

Use a Windows runner/VM where WSL2 tests truly run; never mark release green on skipped mandatory runtime tests.

### Step 4: Build and enforce size locally

```powershell
npm run tauri build
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-browser-release.ps1
```

### Step 5: Commit

```powershell
git add THIRD_PARTY_NOTICES.md third-party scripts/verify-browser-release.ps1 package.json .github/workflows
git commit -m "build(release): gate lightpanda runtime licensing and size"
```

---

## Task 17: Version v0.2.0 + complete acceptance matrix

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/tauri.conf.json`
- Modify: any intentionally synchronized frontend/package version metadata
- Modify: user docs/release notes that still describe Playwright browsers

### Step 1: Version only after migration is complete

Set release version to `0.2.0`; do not version earlier while old runtime is active.

### Step 2: Static/build verification

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm test
npm run build
```

If all-features clippy activates unsupported platform combinations, use the established Windows CI command and document the exact exception rather than silently removing linting.

### Step 3: Full Rust + real runtime tests

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml --tests
$env:LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS='1'
cargo test --manifest-path src-tauri/Cargo.toml --test browser_runtime_integration --jobs 1 -- --nocapture
Remove-Item Env:LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS
```

### Step 4: Explicit 39-tool gate

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration browser_discovery_exposes_exactly_39_public_browser_tools -- --exact --nocapture
```

Confirm internal upstream admin tools are absent from discovery.

### Step 5: Clean-VM WSL lifecycle acceptance

On Windows x86_64 VM exercise:

1. WSL absent -> UAC-supported `wsl --install --no-distribution`;
2. reboot-required state survives restart;
3. post-reboot rootfs download/checksum/import;
4. exact Lightpanda 0.3.7 launch;
5. no automatic Windows drive mounts;
6. Linux interop disabled;
7. Lightpanda runs as `localworkspaces`;
8. Pause kills all page children;
9. disable kills all page children;
10. repair after deliberately damaged owned Lightpanda binary;
11. reset removes only browser profile/cookie/cache;
12. remove unregisters only ownership-verified app distro;
13. unrelated same-name distro remains untouched.

### Step 6: Final NSIS

```powershell
npm run tauri build
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-browser-release.ps1
```

Required: NSIS <100 MiB, no Node/Playwright/Chromium/Firefox payload, only small runtime manifest/notices bundled, checksum-pinned separate downloads, 39 tools, unsupported operations side-effect-free, zero mandatory runtime-test skips.

### Step 7: Final Git verification + commit

```powershell
git status --short
git diff --check
```

Do not erase unrelated pre-existing changes to force a clean tree.

```powershell
git add src-tauri/Cargo.toml src-tauri/tauri.conf.json package.json README.md docs
git commit -m "release: prepare v0.2.0 lightpanda browser runtime"
```

---

## Final acceptance checklist

- [ ] Settings no longer expose engine/Visible Debug; v5 choices migrate deterministically.
- [ ] Lightpanda exactly `0.3.7` x86_64 Linux, SHA-256 `895339b02205171a181dde743ae0068bb4564884076feac8482baca9c212aa5a`.
- [ ] Ubuntu 24.04 minbase rootfs separately published with immutable URL/SHA, package list, SBOM.
- [ ] WSL2 provisioning idempotent, UAC-respecting, reboot-resumable, collision-safe.
- [ ] Destructive WSL actions require owner/runtime/install_id + expected managed install location.
- [ ] Managed distro disables automount and Windows interop.
- [ ] Lightpanda runs unprivileged with telemetry/core dumps disabled.
- [ ] Browser transport is private native MCP stdio; no Lightpanda HTTP listener.
- [ ] One logical page = one Lightpanda child; sibling survives isolated page crash.
- [ ] Shared storage/cookie profile passes concurrent page test.
- [ ] Opaque refs are page/generation scoped and ref actions recover CSS selectors via `nodeDetails`.
- [ ] Public wait states remain `loading | interactive | complete`.
- [ ] Existing 29 tools remain discoverable; 10 new wrappers bring total to 39.
- [ ] `getEnv`, `getCookies`, `save`, `session_*` never directly model-exposed.
- [ ] Navigation success requires post-`goto` verification.
- [ ] No state-changing call is auto-replayed after process loss.
- [ ] Screenshot/viewport/device/dialog/upload/download/save-download are truthful unsupported operations with zero partial side effects.
- [ ] DevTools unsupported; no visible browser UI claimed.
- [ ] Network/performance/DOM/trace are truthful/degraded, never fake Playwright/CDP semantics.
- [ ] Pause/disable terminate all managed browser children.
- [ ] Desktop setup/repair/reset/remove operate only on verified app-owned runtime.
- [ ] Active release tree has no Playwright/Node/browser-runtime dependency.
- [ ] NSIS <100 MiB.
- [ ] AGPL/source reference + Ubuntu rootfs notices/SBOM pass release verification.
- [ ] Real Windows + WSL2 integration suite has zero release skips.
- [ ] Existing unrelated/uncommitted work is preserved; no destructive Git cleanup used.
