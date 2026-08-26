# Browser Automation and Debugging Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a hermetic, background-first Playwright browser automation/debugging subsystem to Local Workspaces with a dedicated profile, stable `browser_*` MCP tools, DevTools-equivalent diagnostics, workspace-confined file transfer, and a versioned NSIS release that works without system Node or first-run browser downloads.

**Architecture:** Local Workspaces remains the only MCP server exposed to ChatGPT. A Rust `BrowserManager` enforces Browser Automation/Pause/settings, then launches a pinned `@playwright/mcp` child over private stdio using rmcp `TokioChildProcess`; the child owns Playwright/browser state but its upstream tools are never exposed directly. The release stages its own Node runtime, Playwright MCP packages, matching Chromium/Firefox browser binaries, profiles, and artifacts; Rust validates URLs/filesystem bridges, maps upstream failures to stable errors, redacts audit data, and returns page screenshots as MCP image content.

**Tech Stack:** Rust/Tauri 2, rmcp 3.1.x with `transport-child-process`, Tokio, Playwright MCP `0.0.79`, its lockfile-pinned Playwright/Playwright Core dependency, embedded Node.js `24.19.0` x64 on Windows, React/TypeScript, PowerShell runtime staging, NSIS.

**Spec:** `docs/superpowers/specs/2026-08-26-browser-automation-debugging-design.md`

## Global Constraints

- Dedicated automation profiles only; never attach to or copy the user's personal Chrome/Edge/Firefox profile.
- `browser_automation_enabled` defaults to `true` in AppSettings v5, per the approved product decision.
- `BrowserEngine::BundledChromium` and `BrowserRunMode::Background` are the defaults.
- `browser_get_status` is the only `browser_*` tool callable while Browser Automation is OFF; it must not start or inspect a page.
- Every browser operation that starts, reads, or interacts with a page is denied with `BROWSER_AUTOMATION_DISABLED` while the toggle is OFF.
- Pause always wins with `ACCESS_PAUSED`; pausing also stops the active browser child best-effort after setting the kill switch.
- Browser page screenshots do not depend on OS `screen_capture_enabled`.
- `file://` navigation is denied in the first release.
- Upload/download bridges are validated in Rust; no arbitrary filesystem path reaches the browser.
- The upstream Playwright MCP child is private stdio-only. No listening upstream MCP port is created.
- The child process cwd is always the active session artifact sandbox; Playwright filesystem roots and any accidental relative output therefore remain inside that sandbox.
- Never expose upstream `browser_run_code` to ChatGPT and never pass model-supplied code into it. A compile-time constant invocation is allowed only for internal Chromium CDP performance metrics.
- User-facing `browser_evaluate` maps only to upstream page-context `browser_evaluate`.
- No system Node dependency and no browser download at application runtime.
- Pin `@playwright/mcp` exactly to `0.0.79`; let `package-lock.json` pin its exact Playwright/Playwright Core dependency, and install browser binaries from that locked Playwright version.
- Stage Node.js `24.19.0` Windows x64 during the build and verify the archive against the official Node SHASUMS256 file before extraction.
- Stage matching Playwright Chromium and Firefox at build time; Chrome/Edge use installed Playwright channels when selected.
- Background launches must not create a visible top-level browser window.
- Visible Debug Chromium/Chrome/Edge launches use headed mode with `--auto-open-devtools-for-tabs`; Firefox returns unsupported for real DevTools UI.
- Profiles are per engine under `<app-data>/browser/profiles/<browser-engine>/` and are never shared across engine formats.
- Device emulation may restart the browser context; when it does, return a new `session_id` and never replay earlier interactions.
- Default action timeout: 10,000 ms. Default navigation timeout: 60,000 ms. Maximum caller-requested timeout: 120,000 ms.
- Valid viewport dimensions are 1 through 8192 pixels inclusive.
- Maximum `browser_run_steps` length: 20 actions.
- Console/network list default limit: 100 entries; maximum: 500 entries.
- Maximum returned snapshot text: 256 KiB, with explicit truncation metadata.
- Artifact retention constants: 10 completed sessions, 7 days, 512 MiB total; cleanup runs on startup and browser close.
- Browser audit is fail-closed and metadata-only: never persist filled text/passwords, cookies, Authorization headers, browser storage values, response bodies, screenshot bytes, trace contents, or download contents.
- Release build increments app version before `npx tauri build`; the next release from 0.1.1 is 0.1.2 unless another version bump lands first.
- Preserve unrelated user changes and do not push unless explicitly asked.

---

### Task 1: AppSettings v5 browser configuration and stable error codes

**Files:**
- Modify: `crates/core/src/error.rs`
- Modify: `src-tauri/src/settings.rs`
- Modify: `src/lib/types.ts`

**Interfaces:**
- Produces Rust `BrowserEngine::{BundledChromium, Chrome, Edge, Firefox}` serialized as `bundled_chromium|chrome|edge|firefox`.
- Produces Rust `BrowserRunMode::{Background, VisibleDebug}` serialized as `background|visible_debug`.
- Produces AppSettings v5 fields `browser_automation_enabled`, `browser_engine`, `browser_run_mode`.
- Produces the stable `ErrorCode` variants listed in the spec.

- [ ] **Step 1: Add failing settings migration and round-trip tests**

In `src-tauri/src/settings.rs`, add tests that a v4 JSON file upgrades to v5 with browser defaults and that all new values round-trip:

```rust
#[test]
fn v4_file_upgrades_to_v5_browser_defaults() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("settings.json"),
        r#"{"version":4,"agent_mode":"autonomous","screen_capture_enabled":false,"capture_scope":"windows_and_displays","visual_verification":true}"#,
    )
    .unwrap();

    let loaded = load_app_settings(dir.path()).unwrap();
    assert_eq!(loaded.version, 5);
    assert!(loaded.browser_automation_enabled);
    assert_eq!(loaded.browser_engine, BrowserEngine::BundledChromium);
    assert_eq!(loaded.browser_run_mode, BrowserRunMode::Background);
}

#[test]
fn browser_settings_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let settings = AppSettings {
        version: 5,
        browser_automation_enabled: false,
        browser_engine: BrowserEngine::Firefox,
        browser_run_mode: BrowserRunMode::VisibleDebug,
        ..AppSettings::default()
    };
    save_app_settings(dir.path(), &settings).unwrap();
    assert_eq!(load_app_settings(dir.path()).unwrap(), settings);
}
```

- [ ] **Step 2: Add failing stable error-code assertions**

Extend the core stable-code test to require exactly these new strings:

```text
BROWSER_AUTOMATION_DISABLED
BROWSER_NOT_RUNNING
BROWSER_START_FAILED
BROWSER_PROCESS_LOST
BROWSER_TARGET_NOT_FOUND
BROWSER_STALE_REF
BROWSER_NAVIGATION_TIMEOUT
BROWSER_ACTION_TIMEOUT
BROWSER_FEATURE_UNSUPPORTED
BROWSER_FEATURE_UNSUPPORTED_IN_MODE
BROWSER_INVALID_VIEWPORT
BROWSER_UPLOAD_DENIED
BROWSER_DOWNLOAD_NOT_FOUND
BROWSER_DOWNLOAD_DENIED
BROWSER_ARTIFACT_FAILED
BROWSER_PROTOCOL_ERROR
```

- [ ] **Step 3: Run the focused tests and verify RED**

Run:

```text
cargo test -p local-workspace-mcp-manager settings::tests::v4_file_upgrades_to_v5_browser_defaults --lib
cargo test -p core error::tests::error_codes_are_stable_strings --lib
```

Expected: compile/test failure because browser enums/fields/errors do not exist.

- [ ] **Step 4: Implement AppSettings v5 and TypeScript mirrors**

Add:

```rust
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserEngine {
    #[default]
    BundledChromium,
    Chrome,
    Edge,
    Firefox,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserRunMode {
    #[default]
    Background,
    VisibleDebug,
}
```

Extend `AppSettings` and `Default`:

```rust
#[serde(default = "default_true")]
pub browser_automation_enabled: bool,
#[serde(default)]
pub browser_engine: BrowserEngine,
#[serde(default)]
pub browser_run_mode: BrowserRunMode,
```

Set `version: 5`, and force `settings.version = 5` on versioned load. Extend `src/lib/types.ts` with matching string unions and fields.

- [ ] **Step 5: Add the browser `ErrorCode` variants and mappings**

Add one enum variant per required stable browser code and map each variant to the exact uppercase string above in `as_str()`.

- [ ] **Step 6: Re-run focused tests and confirm GREEN**

Run:

```text
cargo test -p local-workspace-mcp-manager settings::tests:: --lib
cargo test -p core error::tests::error_codes_are_stable_strings --lib
npm run build
```

Expected: PASS.

- [ ] **Step 7: Commit**

```text
git add crates/core/src/error.rs src-tauri/src/settings.rs src/lib/types.ts
git commit -m "feat(browser): add browser settings and errors"
```

---

### Task 2: Browser domain model, validation, redaction, and artifact policy

**Files:**
- Create: `src-tauri/src/browser/mod.rs`
- Create: `src-tauri/src/browser/types.rs`
- Create: `src-tauri/src/browser/policy.rs`
- Create: `src-tauri/src/browser/artifacts.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces constants `DEFAULT_ACTION_TIMEOUT_MS`, `DEFAULT_NAVIGATION_TIMEOUT_MS`, `MAX_TOOL_TIMEOUT_MS`, `MAX_VIEWPORT_EDGE`, `MAX_BATCH_STEPS`, `DEFAULT_EVENT_LIMIT`, `MAX_EVENT_LIMIT`, `MAX_SNAPSHOT_BYTES`.
- Produces `BrowserRuntimeSettings`, `BrowserLaunchSpec`, `BrowserSessionStatus`, `BrowserImage`, `BrowserArtifact`, `BrowserDownload`.
- Produces `validate_navigation_url`, `validate_viewport`, `validate_timeout`, `bounded_event_limit`, `redacted_origin`.
- Produces `ArtifactStore` rooted at `<app-data>/browser/artifacts` with deterministic cleanup.

- [ ] **Step 1: Write failing policy tests**

Create tests in `browser/policy.rs` for these exact contracts:

```rust
#[test]
fn url_policy_accepts_http_https_and_localhost() {
    assert!(validate_navigation_url("https://example.com/a?q=1").is_ok());
    assert!(validate_navigation_url("http://127.0.0.1:5173/").is_ok());
    assert!(validate_navigation_url("http://localhost:3000/").is_ok());
}

#[test]
fn url_policy_rejects_file_and_non_http_schemes() {
    assert_eq!(validate_navigation_url("file:///C:/secret.txt").unwrap_err().code, ErrorCode::InvalidArgument);
    assert_eq!(validate_navigation_url("javascript:alert(1)").unwrap_err().code, ErrorCode::InvalidArgument);
}

#[test]
fn viewport_is_bounded() {
    assert!(validate_viewport(1, 1).is_ok());
    assert!(validate_viewport(8192, 8192).is_ok());
    assert_eq!(validate_viewport(0, 600).unwrap_err().code, ErrorCode::BrowserInvalidViewport);
    assert_eq!(validate_viewport(9000, 600).unwrap_err().code, ErrorCode::BrowserInvalidViewport);
}
```

Add timeout/list-limit tests for 10,000/60,000 defaults, 120,000 maximum, event limit 100 default/500 maximum, and a 20-step batch limit.

- [ ] **Step 2: Write failing artifact retention tests**

Use temp directories and synthetic timestamps/sizes to assert cleanup retains no more than 10 completed session directories, drops entries older than 7 days, and prunes oldest completed sessions until total retained bytes are <= 512 MiB. Exported workspace files are outside the artifact root and therefore never considered for deletion.

- [ ] **Step 3: Run browser module tests and confirm RED**

Run:

```text
cargo test -p local-workspace-mcp-manager browser:: --lib
```

Expected: compile failure because the module/types/functions do not exist.

- [ ] **Step 4: Implement the domain types and constants**

Define:

```rust
pub const DEFAULT_ACTION_TIMEOUT_MS: u64 = 10_000;
pub const DEFAULT_NAVIGATION_TIMEOUT_MS: u64 = 60_000;
pub const MAX_TOOL_TIMEOUT_MS: u64 = 120_000;
pub const MAX_VIEWPORT_EDGE: u32 = 8_192;
pub const MAX_BATCH_STEPS: usize = 20;
pub const DEFAULT_EVENT_LIMIT: usize = 100;
pub const MAX_EVENT_LIMIT: usize = 500;
pub const MAX_SNAPSHOT_BYTES: usize = 256 * 1024;
pub const MAX_COMPLETED_ARTIFACT_SESSIONS: usize = 10;
pub const MAX_ARTIFACT_AGE_DAYS: i64 = 7;
pub const MAX_ARTIFACT_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
```

Use `BrowserRuntimeSettings { enabled, engine, run_mode }` and `BrowserLaunchSpec { session_id, engine, run_mode, profile_dir, artifact_dir, action_timeout_ms, navigation_timeout_ms, emulation }`.

- [ ] **Step 5: Implement validators and safe URL-origin audit metadata**

`validate_navigation_url` parses with `url::Url`, accepts only `http` and `https`, and never returns a normalized `file:` URL. `redacted_origin` returns only scheme + host + explicit port, dropping path/query/fragment.

- [ ] **Step 6: Implement `ArtifactStore` and cleanup**

Create session directories only beneath the canonical artifact root. Reject any joined/canonicalized path that escapes the root. Cleanup walks only direct session children and applies age/count/size limits using one centrally tested ordering (oldest completed sessions first).

- [ ] **Step 7: Re-run focused tests and confirm GREEN**

Run:

```text
cargo test -p local-workspace-mcp-manager browser:: --lib
cargo fmt --all -- --check
```

Expected: PASS.

- [ ] **Step 8: Commit**

```text
git add src-tauri/src/browser src-tauri/src/lib.rs
git commit -m "feat(browser): add browser domain policy"
```

---

### Task 3: Hermetic Playwright/Node/browser runtime staging

**Files:**
- Modify: `package.json`
- Modify: `package-lock.json`
- Modify: `.gitignore`
- Create: `scripts/build-browser-runtime.ps1`
- Create: `scripts/verify-browser-runtime.ps1`
- Modify: `src-tauri/tauri.conf.json`

**Interfaces:**
- Produces generated `src-tauri/browser-runtime/` containing `node/`, `node_modules/`, `browsers/`, `licenses/`, and `runtime-manifest.json`.
- Manifest fields: `node_version`, `playwright_mcp_version`, `playwright_version`, `playwright_core_version`, `platform`, `browser_entries`.
- Runtime location is generated/build output and is not committed.

- [ ] **Step 1: Pin Playwright MCP and add staging scripts**

Add exact dev dependency:

```json
"@playwright/mcp": "0.0.79"
```

Add scripts:

```json
"build:browser-runtime": "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-browser-runtime.ps1",
"verify:browser-runtime": "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-browser-runtime.ps1"
```

Run `npm install` so `package-lock.json` captures the exact `playwright` and `playwright-core` versions required by 0.0.79.

- [ ] **Step 2: Add runtime/cache ignore rules**

Add:

```text
.cache/browser-runtime/
src-tauri/browser-runtime/
```

Do not ignore the staging scripts.

- [ ] **Step 3: Write `verify-browser-runtime.ps1` before the builder**

The verifier must fail unless all of these exist and agree:

```text
src-tauri/browser-runtime/node/node.exe
src-tauri/browser-runtime/node_modules/@playwright/mcp/cli.js
src-tauri/browser-runtime/node_modules/playwright/package.json
src-tauri/browser-runtime/node_modules/playwright-core/package.json
src-tauri/browser-runtime/browsers/
src-tauri/browser-runtime/runtime-manifest.json
```

It parses package JSON and manifest, requires MCP version `0.0.79`, requires Node version `24.19.0`, requires manifest Playwright versions to equal staged package versions, requires at least one Chromium entry and one Firefox entry, then executes:

```powershell
& $node $mcpCli --help | Out-Null
if ($LASTEXITCODE -ne 0) { throw "staged Playwright MCP CLI failed" }
```

- [ ] **Step 4: Run verifier and confirm RED**

Run:

```text
npm run verify:browser-runtime
```

Expected: FAIL because the runtime is not staged.

- [ ] **Step 5: Implement `build-browser-runtime.ps1`**

The script must:

1. resolve repo/runtime/cache paths;
2. require `node_modules/@playwright/mcp/package.json` version exactly `0.0.79`;
3. read the installed `playwright` and `playwright-core` versions from `node_modules` and never substitute `latest`;
4. download `https://nodejs.org/dist/v24.19.0/node-v24.19.0-win-x64.zip` and `https://nodejs.org/dist/v24.19.0/SHASUMS256.txt` into `.cache/browser-runtime/` only when absent;
5. verify the zip SHA-256 against the official checksum file before extracting;
6. recreate a temporary staging directory;
7. copy Node runtime + Node license files;
8. copy `@playwright/mcp`, `playwright`, and `playwright-core` package directories into staged `node_modules`;
9. set `PLAYWRIGHT_BROWSERS_PATH=<stage>/browsers` and execute the staged Node against `node_modules/playwright/cli.js install chromium firefox`;
10. copy Apache/Microsoft/Playwright licenses into `licenses/`;
11. write `runtime-manifest.json` using the resolved exact package versions and discovered browser directories;
12. atomically replace `src-tauri/browser-runtime/` with the completed stage;
13. invoke `verify-browser-runtime.ps1` and fail the build if verification fails.

No runtime code may contain a downloader.

- [ ] **Step 6: Stage the runtime and confirm GREEN**

Run:

```text
npm run build:browser-runtime
npm run verify:browser-runtime
```

Expected: PASS; manifest reports Node 24.19.0, MCP 0.0.79, matching locked Playwright packages, and Chromium + Firefox browser entries.

- [ ] **Step 7: Add staged runtime to Tauri build/bundle**

Change both build hooks to stage the browser before frontend packaging:

```json
"beforeDevCommand": "npm run build:sidecar && npm run build:browser-runtime && npm run dev",
"beforeBuildCommand": "npm run build:sidecar && npm run build:browser-runtime && npm run build"
```

Add `browser-runtime/**/*` to `bundle.resources` without removing the existing Tailscale license resource.

- [ ] **Step 8: Verify packaged-resource configuration**

Run:

```text
npm run build
npm run verify:browser-runtime
```

Expected: PASS.

- [ ] **Step 9: Commit**

```text
git add package.json package-lock.json .gitignore scripts/build-browser-runtime.ps1 scripts/verify-browser-runtime.ps1 src-tauri/tauri.conf.json
git commit -m "build(browser): stage hermetic playwright runtime"
```

---

### Task 4: Private stdio Playwright MCP driver and Windows Job Object containment

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/process_job.rs`
- Create: `src-tauri/src/browser/driver.rs`
- Create: `src-tauri/src/browser/playwright.rs`
- Modify: `src-tauri/src/browser/mod.rs`

**Interfaces:**
- `BrowserDriver` is injectable for tests.
- `PlaywrightMcpDriver` is the production implementation.
- Upstream generic calls are restricted to a fixed allowlist; `browser_run_code` is absent from that generic allowlist.
- A dedicated `chromium_performance_metrics()` method is the only path allowed to invoke upstream `browser_run_code`, and its code string is compile-time constant.

- [ ] **Step 1: Enable rmcp child-process transport and write PID-containment test**

Add `transport-child-process` to the existing rmcp features.

Add to `JobHandle`:

```rust
pub(crate) fn assign_pid(&self, pid: u32) -> Result<(), String>
```

Implement with `OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid)` then `AssignProcessToJobObject`, closing the process handle afterward. Extend the existing Job Object test to spawn a sleeper, call `assign_pid(child.id())`, drop the job, and prove the process dies.

- [ ] **Step 2: Define the driver interface and upstream result type**

Create:

```rust
#[async_trait::async_trait]
pub trait BrowserDriver: Send + Sync {
    async fn start(&self, spec: BrowserLaunchSpec) -> Result<BrowserDriverStatus, CoreError>;
    async fn stop(&self) -> Result<(), CoreError>;
    async fn call(
        &self,
        tool: &'static str,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> Result<rmcp::model::CallToolResult, CoreError>;
    async fn chromium_performance_metrics(&self) -> Result<serde_json::Value, CoreError>;
}
```

`BrowserDriverStatus` includes running, actual engine/channel, sidecar version, browser version when known, and child PID only for local diagnostics (never remote MCP output unless intentionally sanitized).

- [ ] **Step 3: Write failing allowlist/error-mapping tests**

Require the generic allowlist to include only required upstream tools such as:

```text
browser_navigate
browser_navigate_back
browser_navigate_forward
browser_reload
browser_snapshot
browser_click
browser_hover
browser_drag
browser_type
browser_select_option
browser_press_key
browser_handle_dialog
browser_file_upload
browser_resize
browser_take_screenshot
browser_console_messages
browser_network_requests
browser_evaluate
browser_tabs
browser_wait_for
browser_close
browser_start_tracing
browser_stop_tracing
```

Assert `browser_run_code` is rejected by generic `call()` with `BROWSER_PROTOCOL_ERROR` before any transport call.

- [ ] **Step 4: Implement launch-config JSON generation**

For Bundled Chromium Background, generate a session config equivalent to:

```json
{
  "browser": {
    "browserName": "chromium",
    "userDataDir": "<profile>",
    "launchOptions": { "headless": true },
    "contextOptions": { "acceptDownloads": true }
  },
  "capabilities": ["core", "tracing"],
  "outputDir": "<artifact-dir>",
  "outputMaxSize": 536870912,
  "imageResponses": "allow",
  "allowUnrestrictedFileAccess": false,
  "codegen": "none",
  "timeouts": { "action": 10000, "navigation": 60000, "expect": 5000 }
}
```

Use the exact capability spelling supported by the pinned 0.0.79 runtime after inspecting its discovered tool list; the generated config test must assert that `browser_start_tracing` and `browser_stop_tracing` are discoverable. Chrome maps to `browserName=chromium, channel=chrome`; Edge to `channel=msedge`; Firefox to `browserName=firefox`. Visible Debug uses `headless=false`; Chromium-family additionally receives `--auto-open-devtools-for-tabs`.

- [ ] **Step 5: Implement `PlaywrightMcpDriver::start` over private child stdio**

Resolve the staged runtime from Tauri resources in production and an injected runtime root in tests. Build `tokio::process::Command` for:

```text
<runtime>/node/node.exe <runtime>/node_modules/@playwright/mcp/cli.js --config <session-config.json>
```

Set `PLAYWRIGHT_BROWSERS_PATH=<runtime>/browsers`; set the child `current_dir` to the active session artifact directory; pipe stdin/stdout; redirect stderr to a bounded diagnostic sink; on Windows set `CREATE_NO_WINDOW` for Background mode before spawn. With `allowUnrestrictedFileAccess=false`, this cwd is also the upstream MCP filesystem root, so staged uploads and any unexpected relative output remain inside the session sandbox.

Create `TokioChildProcess`, obtain its PID, create a `JobHandle`, assign the PID, then initialize:

```rust
let client: rmcp::service::RunningService<rmcp::service::RoleClient, ()> =
    ().serve(transport).await.map_err(map_start_error)?;
```

Immediately call `list_all_tools()` and require the upstream allowlist dependencies expected for the configured capabilities. Store the `RunningService` plus `JobHandle` under `tokio::sync::Mutex` so the process tree remains contained.

- [ ] **Step 6: Implement generic calls and static CDP performance call**

Generic `call()` rejects names outside the allowlist. Convert upstream `is_error=true`, transport closure, and timeout text into stable `CoreError` without returning stack traces/internal paths.

`chromium_performance_metrics()` may call upstream `browser_run_code` only with this app-owned constant, never with caller text:

```javascript
async (page) => {
  const session = await page.context().newCDPSession(page);
  await session.send('Performance.enable');
  const result = await session.send('Performance.getMetrics');
  await session.detach();
  return result;
}
```

Return `BROWSER_FEATURE_UNSUPPORTED` for Firefox before making that call.

- [ ] **Step 7: Run driver/process tests and confirm GREEN**

Run:

```text
cargo test -p local-workspace-mcp-manager process_job::tests:: --lib
cargo test -p local-workspace-mcp-manager browser::driver --lib
cargo test -p local-workspace-mcp-manager browser::playwright --lib
```

Expected: PASS without requiring a browser launch for unit-only tests.

- [ ] **Step 8: Commit**

```text
git add src-tauri/Cargo.toml Cargo.lock src-tauri/src/process_job.rs src-tauri/src/browser
git commit -m "feat(browser): add private playwright mcp driver"
```

---

### Task 5: BrowserManager lifecycle, settings synchronization, and Pause shutdown

**Files:**
- Create: `src-tauri/src/browser/manager.rs`
- Modify: `src-tauri/src/browser/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/server.rs`

**Interfaces:**
- Produces `BrowserManager::new(core, app_data_dir, runtime_root, settings, driver)`.
- Produces async `status`, `open`, `close`, `apply_settings`, `on_pause_changed`, `call_upstream`, and `restart_for_emulation` methods.
- `WorkspaceServer` receives `Arc<BrowserManager>`.

- [ ] **Step 1: Write failing manager lifecycle tests with a fake driver**

Create a fake driver recording `start`, `stop`, and tool calls. Assert:

```rust
// status remains available while disabled
assert!(!manager.status().await.enabled);

// page operations are denied while disabled
assert_eq!(manager.open(None).await.unwrap_err().code, ErrorCode::BrowserAutomationDisabled);

// Pause outranks enabled state
core.pause.set(true);
assert_eq!(manager.open(None).await.unwrap_err().code, ErrorCode::AccessPaused);
```

Also assert disabling while running calls driver `stop()` once, changing engine/run mode while running stops the current session, and a subsequent open gets a new session id.

- [ ] **Step 2: Run focused manager tests and confirm RED**

Run:

```text
cargo test -p local-workspace-mcp-manager browser::manager --lib
```

Expected: compile failure because manager does not exist.

- [ ] **Step 3: Implement lifecycle state and guards**

Store runtime settings under `RwLock`, active session/emulation under async state, and driver as `Arc<dyn BrowserDriver>`.

`guard_page_operation()` must check in this order:

```rust
if self.core.pause.is_paused() { return Err(ACCESS_PAUSED); }
if !settings.enabled { return Err(BROWSER_AUTOMATION_DISABLED); }
```

`status()` is an internal policy-neutral snapshot used by the desktop UI and never starts the driver. The MCP `browser_get_status` handler checks Pause first, but deliberately allows the Browser Automation OFF state so it can report `enabled=false` without page access.

- [ ] **Step 4: Make settings changes deterministic**

`apply_settings(new)` behavior:

1. write the new in-memory settings first;
2. if `enabled` changed true->false, stop active driver/session;
3. if engine or run mode changed while running, stop active driver/session;
4. do not auto-open a replacement session;
5. return final status.

This means an OFF toggle blocks new calls even if child shutdown encounters an error.

- [ ] **Step 5: Load AppSettings before starting the public MCP server**

Refactor Tauri setup order to:

```text
app-data dir -> core -> load AppSettings -> AgentPolicyState::from_settings
-> BrowserManager from persisted settings -> start MCP server -> manage state
```

Do not start MCP with default browser/agent policy and patch it later.

- [ ] **Step 6: Synchronize desktop settings and Pause with BrowserManager**

Make `update_app_settings` async. Persist settings through the existing shared settings controller first, then apply browser runtime settings, then sync autostart/UI/unrestricted flags. If browser apply fails after persistence, return a stable settings/runtime error but keep the persisted settings as source of truth; the next browser operation must obey the stored OFF/engine/mode value.

Make `set_paused` async. Set `core.pause` first, emit/update UI/tray, then `await browser_manager.on_pause_changed(paused)`. `on_pause_changed(true)` stops the browser best-effort; even if stop fails, the already-set Pause flag blocks further calls.

- [ ] **Step 7: Inject BrowserManager into `WorkspaceServer` and test helpers**

Change constructor shape to include:

```rust
pub fn new(
    core: Arc<CoreServices>,
    agent_policy: Arc<AgentPolicyState>,
    agent_settings: Arc<AgentSettingsController>,
    capture_provider: Arc<dyn ScreenCaptureProvider>,
    browser: Arc<BrowserManager>,
) -> Self
```

Update production `serve` and test constructors with fake/in-memory browser drivers.

- [ ] **Step 8: Re-run focused + existing MCP tests**

Run:

```text
cargo test -p local-workspace-mcp-manager browser::manager --lib
cargo test -p local-workspace-mcp-manager --test mcp_integration --jobs 1
```

Expected: PASS; existing 14 tools still behave identically before browser handlers are added.

- [ ] **Step 9: Commit**

```text
git add src-tauri/src/browser src-tauri/src/lib.rs src-tauri/src/commands.rs src-tauri/src/server.rs src-tauri/tests/mcp_integration.rs
git commit -m "feat(browser): manage browser lifecycle and policy"
```

---

### Task 6: MCP lifecycle, tabs, navigation, and search tools

**Files:**
- Create: `src-tauri/tests/browser_mcp_integration.rs`
- Modify: `src-tauri/src/browser/types.rs`
- Create: `src-tauri/src/browser/audit.rs`
- Modify: `src-tauri/src/browser/mod.rs`
- Modify: `src-tauri/src/server.rs`

**Interfaces:**
- Adds `browser_get_status`, `browser_open`, `browser_close`, `browser_tabs`, `browser_navigate`, `browser_search`.
- Exposes Local Workspaces opaque `page_id` values rather than upstream Playwright tab indices; ids are session-scoped and reconciled after each tab mutation/list.
- Adds one shared audited browser-handler helper that stores only safe metadata.

- [ ] **Step 1: Create a fake-browser MCP server fixture and write discovery tests**

The fixture constructs a real rmcp `WorkspaceServer` with a fake `BrowserDriver`. Assert tool discovery includes the six new tools in addition to the existing tools.

- [ ] **Step 2: Write disabled/Pause/status tests**

Require:

```text
browser_get_status while disabled -> success with enabled=false and running=false
browser_get_status while paused -> ACCESS_PAUSED
browser_open while disabled -> BROWSER_AUTOMATION_DISABLED
browser_navigate while disabled -> BROWSER_AUTOMATION_DISABLED
browser_open while paused -> ACCESS_PAUSED
browser_tabs while paused -> ACCESS_PAUSED
```

Each handler call produces exactly one audit row; the status row may record enabled/running but no URL content when no page was read.

- [ ] **Step 3: Write URL and search mapping tests**

Assert:

```text
browser_navigate https://example.com/a?secret=x -> driver gets browser_navigate URL, audit stores only https://example.com
browser_navigate file:///C:/secret.txt -> INVALID_ARGUMENT before driver call
browser_search "rust tauri mcp" -> driver gets browser_navigate with an encoded https://www.google.com/search?q=... URL
```

- [ ] **Step 4: Run focused MCP tests and confirm RED**

Run:

```text
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration lifecycle_ --jobs 1
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration navigation_ --jobs 1
```

Expected: FAIL because handlers are absent.

- [ ] **Step 5: Implement argument/result schemas and safe audit helper**

Use explicit enums such as:

```rust
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserTabAction { List, New, Select, Close }

#[derive(Deserialize, JsonSchema)]
pub struct BrowserTabsArgs {
    pub action: BrowserTabAction,
    pub page_id: Option<String>,
    pub url: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNavigationAction { Url, Back, Forward, Reload }
```

`BrowserManager` maintains `page_id -> upstream tab index` only for the current `session_id`. A tab list or mutation reconciles the mapping and returns `BrowserPageSummary { page_id, title, url, active }`. Closing an earlier tab updates remaining mappings; an id that can no longer be reconciled returns `BROWSER_TARGET_NOT_FOUND` instead of guessing an index.

Audit helper accepts an already-redacted detail object and never accepts raw form text or arbitrary upstream result blobs.

- [ ] **Step 6: Implement lifecycle/navigation/search handlers**

Mappings:

```text
browser_open(url?) -> BrowserManager::open; optional URL is validated first; after child start call upstream browser_tabs list to force/reconcile the initial page, then navigate when URL was supplied
browser_close -> BrowserManager::close
browser_tabs list/new/select/close -> reconcile local page ids around upstream browser_tabs; select/close resolve page_id to the current upstream index
browser_navigate url -> browser_navigate
browser_navigate back -> browser_navigate_back
browser_navigate forward -> browser_navigate_forward
browser_navigate reload -> browser_reload
browser_search -> encoded Google URL -> browser_navigate
```

Normalize upstream response into Local Workspaces `BrowserSessionStatus`/tab output instead of exposing raw internal config paths.

- [ ] **Step 7: Re-run tests and confirm GREEN**

Run:

```text
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration --jobs 1
cargo test -p local-workspace-mcp-manager --test mcp_integration --jobs 1
```

Expected: PASS.

- [ ] **Step 8: Commit**

```text
git add src-tauri/src/browser src-tauri/src/server.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "feat(browser): add lifecycle and navigation tools"
```

---

### Task 7: Semantic page interaction and stale-ref recovery tools

**Files:**
- Modify: `src-tauri/src/browser/types.rs`
- Modify: `src-tauri/src/browser/manager.rs`
- Modify: `src-tauri/src/browser/playwright.rs`
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/tests/browser_mcp_integration.rs`

**Interfaces:**
- Adds `browser_snapshot`, `browser_click`, `browser_fill`, `browser_press`, `browser_select`, `browser_hover`, `browser_drag`, `browser_wait`, `browser_dialog`.
- Adds deterministic Playwright error mapping and bounded snapshot output.

- [ ] **Step 1: Write interaction mapping tests using fake upstream calls**

Require exact mappings:

```text
browser_snapshot -> browser_snapshot {}
browser_click {ref:"e12"} -> browser_click {ref:"e12"}
browser_fill {ref:"e3",text:"secret"} -> browser_type {ref:"e3",text:"secret",slowly:false,submit:false}
browser_press {key:"Enter"} -> browser_press_key {key:"Enter"}
browser_select {ref:"e7",values:["US"]} -> browser_select_option
browser_hover -> browser_hover
browser_drag -> browser_drag
browser_dialog -> browser_handle_dialog
```

Assert audit details for `browser_fill` contain ref + byte/character count only, never the string value.

- [ ] **Step 2: Write stale-ref/error redaction tests**

Feed representative upstream tool-error text such as `Ref e12 not found in the current page snapshot` and assert Local Workspaces returns `BROWSER_STALE_REF`. Feed upstream stack/path text and assert the remote message does not contain `node_modules`, drive-letter paths, or JavaScript stack frames.

- [ ] **Step 3: Write bounded snapshot test**

A fake 300 KiB snapshot must return at most 256 KiB of snapshot text plus `truncated=true`; the audit row records returned byte count only.

- [ ] **Step 4: Write wait-condition tests**

Define:

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserWaitCondition {
    TextPresent { text: String },
    TextAbsent { text: String },
    RefVisible { ref_id: String },
    RefHidden { ref_id: String },
    UrlContains { value: String },
    LoadState { state: BrowserLoadState },
    Delay { milliseconds: u64 },
}
```

Text waits map to `browser_wait_for`. `Delay` validates against the same maximum tool timeout and maps milliseconds to the upstream bounded `browser_wait_for {time:<seconds>}` primitive. Ref/URL/load waits poll `browser_evaluate` every 100 ms until the validated timeout; they never use upstream `browser_run_code`.

- [ ] **Step 5: Run focused interaction tests and confirm RED**

Run:

```text
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration interaction_ --jobs 1
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration wait_ --jobs 1
```

Expected: FAIL because handlers/mapping do not exist.

- [ ] **Step 6: Implement handlers, mappings, and error conversion**

Use refs from `browser_snapshot`; do not add coordinate fallback. `browser_press` is page-level unless an upstream target-specific primitive is available; callers focus/click a ref before page-level key input. Wait polling stops immediately on Pause or disabled capability.

- [ ] **Step 7: Re-run tests and confirm GREEN**

Run:

```text
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration --jobs 1
```

Expected: PASS.

- [ ] **Step 8: Commit**

```text
git add src-tauri/src/browser src-tauri/src/server.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "feat(browser): add semantic interaction tools"
```

---

### Task 8: Responsive emulation and page screenshot tools

**Files:**
- Modify: `src-tauri/src/browser/types.rs`
- Modify: `src-tauri/src/browser/manager.rs`
- Modify: `src-tauri/src/browser/playwright.rs`
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/tests/browser_mcp_integration.rs`

**Interfaces:**
- Adds `browser_set_viewport`, `browser_emulate_device`, `browser_screenshot`.
- Adds `BrowserEmulation` state to the active session.
- Browser screenshots return MCP image content and metadata independently of OS capture permission.

- [ ] **Step 1: Write viewport validation/mapping tests**

Assert 320x568, 375x812, 768x1024, and 1440x900 map to upstream `browser_resize`. Assert width/height 0 or >8192 returns `BROWSER_INVALID_VIEWPORT` before driver invocation.

- [ ] **Step 2: Write device-restart semantics test**

Starting from an active session, call an iPhone-like preset. Assert driver stop/start happens once, the same engine-specific profile path is reused, the context launch spec carries viewport/deviceScaleFactor/mobile/touch/userAgent/orientation, and the returned `session_id` differs from the old one. Assert no old interaction is replayed.

Define the caller schema as either a staged Playwright preset or explicit emulation properties:

```rust
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum BrowserEmulationRequest {
    Preset { name: String, orientation: Option<BrowserOrientation> },
    Explicit {
        width: u32,
        height: u32,
        device_scale_factor: f64,
        is_mobile: bool,
        has_touch: bool,
        user_agent: String,
        orientation: BrowserOrientation,
    },
}
```

For `Preset`, load the exact descriptor by name from the staged Playwright device descriptor source and convert it into `BrowserEmulation`; do not hard-code user-agent strings in Rust. For `Explicit`, validate viewport bounds, `device_scale_factor` in `0.1..=10.0`, and user-agent length <= 4096 bytes before restart. Validate the returned effective state in both paths.

- [ ] **Step 3: Write screenshot response tests**

The fake upstream returns a deterministic PNG image block. Assert Local Workspaces returns an MCP `image/png` block plus metadata `{mode, original_width, original_height, returned_width, returned_height, encoded_bytes, sha256}`. With `max_edge=64`, decode/resize/re-encode using the existing Rust `image` dependency.

Explicitly set OS `screen_capture_enabled=false` in this test and prove browser screenshot still succeeds.

- [ ] **Step 4: Run focused tests and confirm RED**

Run:

```text
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration viewport_ --jobs 1
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration device_ --jobs 1
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration screenshot_ --jobs 1
```

Expected: FAIL before implementation.

- [ ] **Step 5: Implement viewport/device/screenshot tools**

Map screenshots to upstream `browser_take_screenshot` with PNG and viewport/full-page/element-ref modes. Never persist routine screenshots. For device emulation, stop/relaunch using the same per-engine profile and a new session artifact directory; update status with effective emulation.

- [ ] **Step 6: Re-run focused tests and confirm GREEN**

Run the three focused commands plus:

```text
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration --jobs 1
```

Expected: PASS.

- [ ] **Step 7: Commit**

```text
git add src-tauri/src/browser src-tauri/src/server.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "feat(browser): add responsive and screenshot tools"
```

---

### Task 9: Console, network, DOM, evaluate, performance, trace, and visible DevTools

**Files:**
- Modify: `src-tauri/src/browser/types.rs`
- Modify: `src-tauri/src/browser/driver.rs`
- Modify: `src-tauri/src/browser/playwright.rs`
- Modify: `src-tauri/src/browser/manager.rs`
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/tests/browser_mcp_integration.rs`

**Interfaces:**
- Adds `browser_console`, `browser_network`, `browser_dom`, `browser_evaluate`, `browser_performance`, `browser_trace`, `browser_devtools`.
- Network first release returns request/response metadata only and never response bodies/cookies/Authorization headers.

- [ ] **Step 1: Write console/network filtering and redaction tests**

Require default limit 100, max 500. `browser_console` may return message text to the caller but audit only `{count, levels, cursor}`. `browser_network` maps only to upstream `browser_network_requests`; never call richer body/header-fetch tools. Filter returned headers through a deny set containing at least `authorization`, `proxy-authorization`, `cookie`, `set-cookie`, `x-api-key`.

- [ ] **Step 2: Write DOM/evaluate tests**

`browser_dom {ref}` maps to upstream `browser_evaluate` with that exact ref and an app-owned expression returning tagName, role/name-compatible attributes, bounding rect, a bounded allowlist of computed style keys, accessibility-relevant attributes, and bounded outerHTML. Sensitive attributes such as `value` on password fields and token-like attribute names are redacted.

`browser_evaluate {expression, ref?}` maps directly to upstream page-context `browser_evaluate {expression, ref}` and bounds serialized result size. Assert it never routes through `browser_run_code`.

- [ ] **Step 3: Write performance tests including fixed-code boundary**

For all engines, return page Performance API data from app-owned `browser_evaluate` expressions. For Chromium-family engines, additionally call `chromium_performance_metrics()` and merge `Performance.getMetrics` results. Assert the fake driver receives no caller-controlled code in the CDP method. Firefox returns page metrics plus `{cdp_supported:false}` rather than failing the whole tool.

- [ ] **Step 4: Write trace tests**

`browser_trace {action:start|stop|status}` maps start/stop only to upstream `browser_start_tracing`/`browser_stop_tracing`. On stop, scan only the active session trace directory, create opaque artifact ids, and return safe metadata `{artifact_id, files, bytes}` without returning internal absolute paths or trace contents.

- [ ] **Step 5: Write visible DevTools mode tests**

Background -> `browser_devtools` returns `BROWSER_FEATURE_UNSUPPORTED_IN_MODE` without changing the launch. Visible Debug + Firefox -> `BROWSER_FEATURE_UNSUPPORTED`. Visible Debug + Chromium/Chrome/Edge -> success only if current launch spec contains headed mode and `--auto-open-devtools-for-tabs`; the tool reports mode/support state and never opens a second unrelated browser.

- [ ] **Step 6: Run focused tests and confirm RED**

Run:

```text
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration debug_ --jobs 1
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration trace_ --jobs 1
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration devtools_ --jobs 1
```

Expected: FAIL before handlers exist.

- [ ] **Step 7: Implement debug handlers and fixed internal expressions**

Keep all fixed DOM/performance expressions in named `const` strings under `browser/playwright.rs` so tests can prove model input never becomes upstream process-level code. Do not expose upstream storage/cookie tools in the first release.

- [ ] **Step 8: Re-run focused tests and confirm GREEN**

Run the three focused commands and full `browser_mcp_integration`.

- [ ] **Step 9: Commit**

```text
git add src-tauri/src/browser src-tauri/src/server.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "feat(browser): add browser debugging tools"
```

---

### Task 10: Upload/download sandbox and workspace bridge

**Files:**
- Modify: `src-tauri/src/browser/artifacts.rs`
- Create: `src-tauri/src/browser/files.rs`
- Modify: `src-tauri/src/browser/mod.rs`
- Modify: `src-tauri/src/browser/manager.rs`
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/tests/browser_mcp_integration.rs`

**Interfaces:**
- Adds `browser_upload`, `browser_downloads`, `browser_save_download_to_workspace`.
- Uploads from workspace require Read; exports into workspace require Write and normal path confinement.
- Upstream receives only files staged beneath the active session artifact root.

- [ ] **Step 1: Write upload confinement tests**

Test these cases:

```text
workspace file + Read=true -> copied to <session>/upload-staging/<opaque-id> and upstream gets staged absolute path
workspace file + Read=false -> BROWSER_UPLOAD_DENIED
../ traversal -> BROWSER_UPLOAD_DENIED
absolute arbitrary C:\... path from MCP -> BROWSER_UPLOAD_DENIED
browser sandbox artifact id -> allowed if it resolves inside active browser artifact root
```

Do not pass original workspace absolute paths to upstream Playwright.

- [ ] **Step 2: Write download discovery/export tests**

The fake browser creates files only under `<session>/downloads`. `browser_downloads` returns opaque ids, filename, size, completed status, and hash when complete; no absolute path.

`browser_save_download_to_workspace` requires workspace Write, validates the destination with the same canonical confinement logic as file tools, copies atomically, and returns `{workspace_id,path,bytes,sha256}`. Missing ids -> `BROWSER_DOWNLOAD_NOT_FOUND`; read-only workspace/traversal -> `BROWSER_DOWNLOAD_DENIED`.

- [ ] **Step 3: Write audit-redaction tests**

Upload audit records source kind, workspace id/path when applicable, file count and byte counts but not file content. Download audit records opaque id, workspace target and safe size/hash only.

- [ ] **Step 4: Run focused tests and confirm RED**

Run:

```text
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration upload_ --jobs 1
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration download_ --jobs 1
```

Expected: FAIL before bridge implementation.

- [ ] **Step 5: Implement Rust-owned staging/export**

Use the existing workspace repository/canonical path pipeline for permission checks. Stage uploads by copying into `upload-staging`; call upstream `browser_file_upload` only with staged paths. Never set Playwright MCP `allowUnrestrictedFileAccess=true`.

- [ ] **Step 6: Re-run focused tests and confirm GREEN**

Run the two focused commands plus full `browser_mcp_integration`.

- [ ] **Step 7: Commit**

```text
git add src-tauri/src/browser src-tauri/src/server.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "feat(browser): confine browser file transfers"
```

---

### Task 11: Bounded `browser_run_steps` flow execution

**Files:**
- Modify: `src-tauri/src/browser/types.rs`
- Modify: `src-tauri/src/browser/manager.rs`
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/tests/browser_mcp_integration.rs`

**Interfaces:**
- Adds `browser_run_steps` with a tagged action enum limited to existing safe local operations.
- Maximum 20 steps; stop-on-first-failure default; no crash replay.

- [ ] **Step 1: Define and test the allowed batch schema**

Use a closed tagged enum:

```rust
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserStep {
    Navigate { url: String },
    Snapshot,
    Click { ref_id: String },
    Fill { ref_id: String, text: String },
    Press { key: String },
    Select { ref_id: String, values: Vec<String> },
    Wait { condition: BrowserWaitCondition, timeout_ms: Option<u64> },
    SetViewport { width: u32, height: u32 },
    Screenshot { mode: BrowserScreenshotMode, ref_id: Option<String>, max_edge: Option<u32> },
}
```

No evaluate, upload, download-export, dialog, trace, or DevTools action is accepted in a batch.

- [ ] **Step 2: Write limit/failure/no-replay tests**

Assert 21 steps -> `INVALID_ARGUMENT` before the driver. For a 3-step batch where step 2 fails, output includes results for step 1 and `{failed_index:1}` and step 3 is never executed. Simulate process loss on a click and prove the batch does not restart/replay that click.

- [ ] **Step 3: Write audit-redaction test**

Audit contains only `step_count`, ordered step type names, failure index/status, and safe viewport/screenshot metadata. Filled text values must not occur in serialized audit detail.

- [ ] **Step 4: Run focused test and confirm RED**

Run:

```text
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration run_steps_ --jobs 1
```

Expected: FAIL before handler exists.

- [ ] **Step 5: Implement sequential local dispatch**

Dispatch through the same internal manager methods used by individual tools so URL/viewport/Pause/disabled/error rules are identical. Stop on first failure by default; do not create a second authorization path.

- [ ] **Step 6: Re-run focused/full browser MCP tests and confirm GREEN**

Run:

```text
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration --jobs 1
```

Expected: PASS.

- [ ] **Step 7: Commit**

```text
git add src-tauri/src/browser src-tauri/src/server.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "feat(browser): add bounded browser step flows"
```

---

### Task 12: Agent guidance and browser-focused MCP prompt

**Files:**
- Modify: `src-tauri/src/agent_skills.rs`
- Modify: `src-tauri/tests/mcp_integration.rs`

**Interfaces:**
- Existing agent workflow remains `DISCOVER -> PLAN -> INSPECT -> ACT -> VERIFY -> RECOVER -> REPORT`.
- Adds prompt `debug-web`.
- Instructions teach browser evidence/recovery without granting authority.

- [ ] **Step 1: Strengthen instruction/prompt contract tests and confirm RED**

Require instructions to contain browser guidance semantics for:

```text
browser_get_status
browser_snapshot
browser_console
browser_network
browser_screenshot
BROWSER_STALE_REF
BROWSER_AUTOMATION_DISABLED
never bypass browser disable/Pause with run_command or run_powershell
```

Require prompt discovery to add `debug-web` while preserving the five existing prompts.

- [ ] **Step 2: Add browser workflow guidance**

Teach the agent to:

1. read browser status when browser evidence matters;
2. open/navigate only when relevant;
3. snapshot before ref-based interaction;
4. re-snapshot on stale refs;
5. verify UI with DOM/URL/console/network/screenshot evidence;
6. use background debug data instead of visible DevTools unless the user configured Visible Debug;
7. use responsive matrix checks for layout work;
8. never shell around disabled Browser Automation/Pause;
9. never send model code to any process-level Playwright code-execution tool.

- [ ] **Step 3: Add `debug-web` prompt**

The prompt should tell the agent to reproduce the reported web issue, capture semantic/browser-debug evidence, make code changes through workspace tools, rerun the browser flow, and report the evidence that changed.

- [ ] **Step 4: Re-run instruction/prompt tests and confirm GREEN**

Run:

```text
cargo test -p local-workspace-mcp-manager --test mcp_integration server_instructions_define_the_agent_operating_loop --jobs 1
cargo test -p local-workspace-mcp-manager --test mcp_integration prompt_discovery_exposes_the_agent_skill_pack --jobs 1
```

Expected: PASS.

- [ ] **Step 5: Commit**

```text
git add src-tauri/src/agent_skills.rs src-tauri/tests/mcp_integration.rs
git commit -m "feat(agent): teach browser debugging workflow"
```

---

### Task 13: Desktop Browser Automation settings, diagnostics, and profile reset

**Files:**
- Modify: `src/lib/types.ts`
- Modify: `src/api.ts`
- Modify: `src/pages/settings-page.tsx`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Adds Tauri commands `browser_runtime_status` and `reset_browser_profile`.
- Settings UI adds one Browser Automation card with toggle, engine, mode, runtime status, and profile reset.

- [ ] **Step 1: Add TypeScript API shapes first**

Add:

```ts
export type BrowserEngine = "bundled_chromium" | "chrome" | "edge" | "firefox";
export type BrowserRunMode = "background" | "visible_debug";

export interface BrowserRuntimeStatus {
  enabled: boolean;
  running: boolean;
  configured_engine: BrowserEngine;
  run_mode: BrowserRunMode;
  bundled_chromium_available: boolean;
  chrome_available: boolean;
  edge_available: boolean;
  firefox_available: boolean;
  node_version: string | null;
  playwright_mcp_version: string | null;
  playwright_version: string | null;
}
```

Add API wrappers for `browser_runtime_status` and `reset_browser_profile`.

- [ ] **Step 2: Add backend diagnostics/profile-reset commands**

`browser_runtime_status` reads the runtime manifest and probes optional Chrome/Edge channel availability without starting a page. It never returns active URL/history/cookies/download filenames.

`reset_browser_profile(engine)` is desktop-only. It stops the active browser first, canonicalizes exactly `<app-data>/browser/profiles/<engine>/`, deletes only that directory, recreates it empty, audits source=`ui`, and returns runtime status. Reject any arbitrary path input because the command accepts the enum, not a filesystem string.

- [ ] **Step 3: Add the Settings card**

Use the existing card/select/switch styles. Controls:

```text
Browser Automation: ON/OFF
Browser: Bundled Chromium / Chrome / Edge / Firefox
Run mode: Background / Visible Debug
Runtime status: available/running + versions
Profile: "Dedicated Local Workspaces profile" + Reset profile button
```

Disable unavailable browser options based on diagnostics. Show explanatory copy that the profile is separate from personal browser data. Changing engine/mode uses the existing full AppSettings save path; BrowserManager stops any active session and the next open uses the new setting.

- [ ] **Step 4: Add reset confirmation**

Use an AlertDialog that explicitly says resetting removes automation cookies/login/localStorage for the selected engine and does not touch the user's normal browser profile. Confirm calls `resetBrowserProfile(settings.browser_engine)`.

- [ ] **Step 5: Build frontend and run Rust command tests**

Run:

```text
npm run build
cargo test -p local-workspace-mcp-manager commands::tests:: --lib
```

Expected: PASS.

- [ ] **Step 6: Commit**

```text
git add src/lib/types.ts src/api.ts src/pages/settings-page.tsx src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(browser): add browser settings controls"
```

---

### Task 14: Real bundled-browser integration, background-window, and crash recovery tests

**Files:**
- Create: `src-tauri/tests/browser_runtime_integration.rs`
- Create: `src-tauri/tests/fixtures/browser_test_app.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/browser/playwright.rs`
- Modify: `src-tauri/src/browser/manager.rs`

**Interfaces:**
- Test fixture is a deterministic localhost server containing interactive controls, console output, a failing/successful API route, responsive marker, upload input, download route, and rerender/stale-ref trigger.
- Tests execute through the same `BrowserManager`/Playwright MCP child path used in production.

- [ ] **Step 1: Build a deterministic local test web app fixture**

Expose routes:

```text
/ -> page with text input, select, button, dialog trigger, rerender button, responsive marker, upload input, download link
/api/ping -> JSON 200 and console "api-ping-complete"
/api/fail -> JSON 500
/download.txt -> deterministic text attachment
```

The page writes known DOM state after form actions and logs one known console warning/error for assertions.

- [ ] **Step 2: Write real Background lifecycle/navigation/interaction test**

Skip with an explicit message only when `src-tauri/browser-runtime/runtime-manifest.json` is absent in a developer checkout; release verification always stages it first.

Drive:

```text
start Background BundledChromium
navigate localhost fixture
snapshot -> resolve ref
fill/click/select
assert DOM state
assert console known message
assert network /api/ping and /api/fail metadata
browser_evaluate document.title
close cleanly
```

- [ ] **Step 3: Add responsive/screenshot/trace test**

For each viewport `(320,568)`, `(375,812)`, `(768,1024)`, `(1440,900)`, call local viewport logic and prove `window.innerWidth/innerHeight` plus responsive marker state. Capture PNG and validate decoded dimensions/hash. Start/stop trace and prove trace artifact ids resolve only inside the active session trace directory.

- [ ] **Step 4: Add upload/download confinement E2E**

Create a temp approved workspace file, stage/upload it through Rust, verify page reports its filename, trigger deterministic download, list it by opaque id, export it to an approved workspace path, verify bytes/hash, and test one traversal export is denied.

- [ ] **Step 5: Add Background no-visible-window Windows test**

After Background launch, enumerate visible top-level windows for PIDs in the browser Job/process tree using Win32 APIs. Assert no visible top-level window belongs to that tree. Do not use screenshot visibility as the criterion.

- [ ] **Step 6: Add crash/recovery tests**

Expose a test-only method/driver hook to terminate the child/browser process. During a read-only status/snapshot scenario, assert one safe restart can establish a new session. During an ambiguous click/action scenario, assert `BROWSER_PROCESS_LOST` and zero automatic replay count.

- [ ] **Step 7: Add browser compatibility tests**

Mandatory: Bundled Chromium Background. Also run staged Playwright Firefox core navigation/interaction. Chrome/Edge tests are conditional on detected channel availability. Assert `browser_devtools` unsupported for Firefox and available only for Visible Debug Chromium-family.

- [ ] **Step 8: Run real-browser suite**

Run:

```text
npm run build:browser-runtime
cargo test -p local-workspace-mcp-manager --test browser_runtime_integration --jobs 1 -- --nocapture
```

Expected: all mandatory Bundled Chromium + staged Firefox tests PASS; Chrome/Edge either PASS when present or report a deterministic skipped-unavailable test condition.

- [ ] **Step 9: Commit**

```text
git add src-tauri/tests/browser_runtime_integration.rs src-tauri/tests/fixtures/browser_test_app.rs src-tauri/Cargo.toml Cargo.lock src-tauri/src/browser
git commit -m "test(browser): verify real browser automation runtime"
```

---

### Task 15: Documentation, full verification, version 0.1.2, and NSIS release

**Files:**
- Modify: `README.md`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `Cargo.lock`
- Modify only task-related files if verification reveals a defect.

**Interfaces:**
- Release artifact name for this baseline: `Local Workspace MCP Manager_0.1.2_x64-setup.exe`.
- README documents browser capability, background default, dedicated profile, debugging tool families, and installer size impact.

- [ ] **Step 1: Document the browser feature and privacy model**

README must state:

```text
Browser Automation defaults ON but can be disabled with one Settings toggle.
Bundled Chromium + dedicated Local Workspaces profile are the defaults.
Background mode does not open a normal browser window.
Visible Debug can show Chromium-family browser + DevTools.
Browser screenshots are page-renderer captures and are independent of OS Screen Capture permission.
Uploads/downloads remain sandbox/workspace-confined.
Pause blocks browser automation immediately.
```

Do not document a personal-profile attachment mode because it does not exist.

- [ ] **Step 2: Run static/spec hygiene checks**

Run:

```text
git diff --check
```

Search the production browser adapter for `browser_run_code` and verify every occurrence is either the explicit generic-deny rule/test or the single app-owned `chromium_performance_metrics` constant path; there must be no path that interpolates MCP/user input into process-level code.

- [ ] **Step 3: Run complete Rust quality gates**

Run:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --jobs 1
```

Expected: PASS.

- [ ] **Step 4: Run frontend, Tailscale, and browser runtime gates**

Run:

```text
npm run build
npm run build:sidecar
npm run build:browser-runtime
npm run verify:browser-runtime
```

From `sidecars/tailscale-funnel` run:

```text
go test ./...
go vet ./...
```

Expected: PASS.

- [ ] **Step 5: Run real bundled-browser acceptance tests**

Run:

```text
cargo test -p local-workspace-mcp-manager --test browser_runtime_integration --jobs 1 -- --nocapture
```

Expected: mandatory Bundled Chromium and staged Firefox tests PASS, including the 4 responsive viewports, page screenshot, console/network, trace, file bridge, background-window, and crash recovery assertions.

- [ ] **Step 6: Bump release version from 0.1.1 to 0.1.2**

Update exactly:

```text
src-tauri/Cargo.toml package.version -> 0.1.2
src-tauri/tauri.conf.json version -> 0.1.2
```

Run `cargo check -p local-workspace-mcp-manager --tests --jobs 1` so Cargo.lock records 0.1.2.

If repository version has already advanced past 0.1.1 when this task is executed, increment the current patch version by one instead and use that same value in both files; never decrement or reuse an existing installer version.

- [ ] **Step 7: Build NSIS and verify artifact contents/metadata**

Run:

```text
npx tauri build
```

Expected output includes a version-distinct installer under `target/release/bundle/nsis/`.

Use PowerShell to verify:

```powershell
$exe = Get-Item 'target/release/bundle/nsis/Local Workspace MCP Manager_0.1.2_x64-setup.exe'
$hash = (Get-FileHash $exe.FullName -Algorithm SHA256).Hash
$exe | Select-Object FullName, Length, LastWriteTime
$hash
```

Also inspect the produced bundle/resources or install into a temporary acceptance environment and prove Node, Playwright MCP, Chromium, and Firefox runtime assets are included; launch must work with outbound network disabled against the local test fixture.

- [ ] **Step 8: Run final git hygiene**

Run:

```text
git diff --check
git status --short --branch
git diff --stat
```

Confirm generated `src-tauri/browser-runtime/`, `.cache/browser-runtime/`, frontend dist, target artifacts, and installer are untracked/ignored rather than staged.

- [ ] **Step 9: Commit release source changes**

```text
git add README.md src-tauri/Cargo.toml src-tauri/tauri.conf.json Cargo.lock
git commit -m "release: prepare browser automation 0.1.2"
```

If verification required source fixes, commit those fixes in their owning task before this release commit rather than hiding them inside the version bump.

- [ ] **Step 10: Final completion evidence**

Record in the final report:

```text
source commit(s)
exact app version
NSIS full path
installer byte size
installer LastWriteTime
SHA-256
Rust test/clippy/fmt results
frontend result
Tailscale sidecar test/vet result
browser runtime verification result
real Chromium/Firefox acceptance result
final git status
```

Do not push unless the user explicitly requests it.
