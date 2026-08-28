# Lightpanda Native MCP Full Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the current 39-tool Playwright-shaped Browser Automation compatibility layer with exactly 31 `browser_lightpanda_*` tools that expose the full native Lightpanda 0.3.7 MCP tool surface over one private HTTP MCP server with native sessions.

**Architecture:** Keep the existing managed WSL2 provisioner, pinned Lightpanda artifact, Pause gate, Browser Automation toggle, audit database, and desktop runtime management. Replace the stdio/page-driver/tab/ref architecture with an app-owned `reqwest` JSON-RPC client, one `LightpandaNativeService`, explicit `Mcp-Session-Id` routing, raw `backendNodeId`, and thin typed public handlers. The native browser contract is frozen in a reviewed JSON resource and verified against `tools/list` before the runtime is considered usable.

**Tech Stack:** Rust 2021, Tauri 2, Tokio, reqwest 0.12, rmcp 0.17 model/server macros, serde/serde_json, schemars, Axum test servers, managed WSL2, Lightpanda 0.3.7, Vitest/TypeScript/React for desktop UI tests.

**Spec:** `docs/superpowers/specs/2026-08-28-lightpanda-native-mcp-full-parity-design.md`

## Global Constraints

- Lightpanda remains pinned to `0.3.7` x86_64 Linux with SHA-256 `895339b02205171a181dde743ae0068bb4564884076feac8482baca9c212aa5a`.
- Public browser tools are **only** `browser_lightpanda_*`; no legacy `browser_*` aliases survive the final registration switch.
- The public Lightpanda browser tool count is exactly **31** and maps one-to-one to the pinned upstream `tools/list` contract.
- The internal browser transport is one Lightpanda HTTP MCP server bound to `127.0.0.1` inside the managed WSL2 distro; never bind `0.0.0.0` as a fallback.
- Protocol bootstrap, session-control calls, and ordinary calls with omitted `session_id` explicitly send `Mcp-Session-Id: default` so initialization never creates a hidden generated session such as `s1`.
- `initialize.serverInfo.version` is not the binary-version authority. Binary pinning uses the runtime manifest, SHA-256, and `lightpanda version`; native tool/schema parity is verified separately.
- Native named sessions are not re-initialized. After `session_new`, route calls directly with the returned/native session ID header.
- Raw native `backendNodeId` is public and is never converted to `e1/e2/...` refs. Selector targeting remains recommended for action flows intended to be replayable through PandaScript `save`.
- Public native tool argument names preserve upstream spelling and casing (`waitUntil`, `backendNodeId`, `maxBytes`, etc.). The only systematic Local Workspaces addition is optional `session_id` routing metadata on non-session-control tools.
- Upstream `CallToolResult.isError=true` is preserved. Valid upstream JSON-RPC tool errors preserve the native numeric code/message for the authorized caller; malformed transport/protocol data alone maps to `BROWSER_PROTOCOL_ERROR`.
- No failed or uncertain native call is automatically replayed.
- Pause and `browser_automation_enabled` remain authoritative. Browser tooling must not be bypassed through shell/PowerShell/browser side channels.
- Cookies, LP_* env values, fill values, search queries, evaluate scripts, extraction schemas, page text, console text, PandaScript contents, and raw session names/IDs never enter audit rows or desktop diagnostics.
- Lightpanda runs as the unprivileged `localworkspaces` Linux user; telemetry and core dumps remain disabled.
- `browser_lightpanda_save` resolves from `/var/lib/local-workspaces/browser/pandascripts`; Reset Browser Data preserves that directory, while Remove Browser Runtime deletes it with the owned distro.
- `repair_browser_runtime`, `reset_browser_data`, and `remove_browser_runtime` stop the running HTTP server before mutating runtime/browser files.
- Do not add a raw `browser_lightpanda_call(tool,args)` escape hatch.
- Do not add CDP, Playwright/Puppeteer, graphical screenshots, upload/download shims, viewport/device emulation, proxy configuration, network interception, or other out-of-scope Lightpanda platform capabilities.
- Preserve unrelated user changes. Use targeted tests during development; run the full release gates only after the migration is assembled.

---

## Reviewed Code Map and Target File Structure

### Existing files that remain but change responsibility

- `src-tauri/src/browser/runtime.rs` — retain WSL/rootfs/binary provisioning; replace restricted stdio smoke verification with the pinned full-contract HTTP verification path; add PandaScript directory to runtime layout.
- `src-tauri/src/browser/wsl.rs` — retain WSL ownership/isolation; add fixed-working-directory HTTP Lightpanda launch and fixed graceful-stop support.
- `src-tauri/src/browser/lightpanda.rs` — replace `LightpandaPageDriver`/per-page stdio code with `LightpandaNativeService` and the `BrowserNativeCaller` trait.
- `src-tauri/src/browser/mcp.rs` — replace compatibility arguments/results/ref adapters with native 31-tool argument types, routing extraction, and safe audit metadata helpers.
- `src-tauri/src/browser/policy.rs` — retain navigation URL policy; remove compatibility-only viewport/ref/batch validation after callers disappear.
- `src-tauri/src/browser/audit.rs` — retain one-row fail-closed audit helper; adapt native error classification and metadata.
- `src-tauri/src/browser/types.rs` — shrink to runtime settings/status constants/types still used by the native service; remove page/tab/emulation/download compatibility types.
- `src-tauri/src/server.rs` — replace 39 browser handlers with exactly 31 native handlers; hold `Arc<dyn BrowserNativeCaller>` for testability.
- `src-tauri/src/lib.rs` — construct/manage `Arc<LightpandaNativeService>` rather than `BrowserManager`.
- `src-tauri/src/commands.rs` — desktop runtime commands/status use the native service lifecycle.
- `src-tauri/src/agent_skills.rs` — replace legacy browser guidance with native Lightpanda workflow + separate Screen Capture guidance.
- `src/lib/types.ts` and `src/pages/settings-page.tsx` — expose redacted native HTTP/parity status and correct reset/remove copy.
- `src-tauri/tests/browser_mcp_integration.rs` — rewrite around fake `BrowserNativeCaller`, exact tool discovery/schema parity, policy/audit behavior.
- `src-tauri/tests/browser_runtime_integration.rs` — rewrite real Windows/WSL suite around one HTTP server, native sessions, all 31 tools, backend IDs, selectors/PandaScript, lifecycle.
- `scripts/verify-browser-release.ps1` — require the new parity output and new real-runtime coverage instead of the old t01..t19 compatibility count.
- `package.json` — add `verify:lightpanda-contract` and keep `verify:browser-release` as the full release gate that invokes the static contract verifier.

### New files

- `src-tauri/resources/lightpanda-mcp-contract-0.3.7.json` — reviewed upstream `tools/list` snapshot containing exactly 31 names and input schemas.
- `src-tauri/src/browser/contract.rs` — parses/verifies the snapshot and compares discovered native tools/schemas.
- `src-tauri/src/browser/http.rs` — app-owned JSON-RPC HTTP/SSE client with explicit per-request session header selection.
- `scripts/verify-lightpanda-mcp-contract.py` — static repository verifier for the frozen 31-tool contract resource and mapping invariants.

### Files expected to be deleted after the final compatibility switch

- `src-tauri/src/browser/driver.rs`
- `src-tauri/src/browser/manager.rs`
- `src-tauri/src/browser/artifacts.rs` if no non-legacy references remain
- `src-tauri/src/browser/files.rs` if no non-legacy references remain

Do not delete a file until `search_text`/compiler evidence shows it has no remaining required references.

---

### Task 1: Freeze and verify the exact Lightpanda 0.3.7 native MCP contract

**Files:**
- Create: `src-tauri/resources/lightpanda-mcp-contract-0.3.7.json`
- Create: `src-tauri/src/browser/contract.rs`
- Modify: `src-tauri/src/browser/mod.rs`
- Create: `scripts/verify-lightpanda-mcp-contract.py`
- Modify: `package.json`
- Test: `src-tauri/src/browser/contract.rs`

**Interfaces:**
- Produces: `pub const EXPECTED_NATIVE_TOOL_COUNT: usize = 31`
- Produces: `pub const NATIVE_TOOL_MAPPINGS: &[(&str, &str)]`
- Produces: `pub fn pinned_contract() -> Result<LightpandaMcpContract, CoreError>`
- Produces: `pub fn verify_discovered_tools(discovered: &[rmcp::model::Tool]) -> Result<(), CoreError>`; this function compares exact names and input schemas to the pinned snapshot.
- Consumed by Tasks 2, 3, 5, 6, 9, and 11.

- [ ] **Step 1: Write the contract tests first**

Add unit tests that make the target contract explicit before creating the resource:

```rust
#[test]
fn pinned_contract_has_exact_31_native_tools() {
    let c = pinned_contract().unwrap();
    assert_eq!(c.tools.len(), 31);
    let names: std::collections::BTreeSet<_> =
        c.tools.iter().map(|t| t.name.as_str()).collect();
    let expected: std::collections::BTreeSet<_> = [
        "goto", "search", "markdown", "html", "links", "evaluate", "extract",
        "tree", "nodeDetails", "interactiveElements", "structuredData", "detectForms",
        "click", "fill", "scroll", "waitForSelector", "waitForScript", "waitForState",
        "hover", "press", "selectOption", "setChecked", "findElement", "consoleLogs",
        "getUrl", "getCookies", "getEnv", "save", "session_new", "session_list",
        "session_close",
    ].into_iter().collect();
    assert_eq!(names, expected);
}

#[test]
fn mapping_is_one_to_one_and_uses_only_browser_lightpanda_prefix() {
    assert_eq!(NATIVE_TOOL_MAPPINGS.len(), 31);
    let upstream: std::collections::BTreeSet<_> =
        NATIVE_TOOL_MAPPINGS.iter().map(|(u, _)| *u).collect();
    let public: std::collections::BTreeSet<_> =
        NATIVE_TOOL_MAPPINGS.iter().map(|(_, p)| *p).collect();
    assert_eq!(upstream.len(), 31);
    assert_eq!(public.len(), 31);
    assert!(public.iter().all(|name| name.starts_with("browser_lightpanda_")));
}
```

- [ ] **Step 2: Run the contract tests and verify they fail because the module/resource does not exist yet**

Run:

```text
cargo test --manifest-path src-tauri/Cargo.toml browser::contract::tests -- --nocapture
```

Expected: compile/test failure referencing missing `browser::contract`, `pinned_contract`, or contract resource.

- [ ] **Step 3: Add the reviewed contract resource with the exact 31 upstream schemas**

Create `src-tauri/resources/lightpanda-mcp-contract-0.3.7.json` in this shape:

```json
{
  "lightpanda_version": "0.3.7",
  "tool_count": 31,
  "tools": [
    {
      "name": "goto",
      "inputSchema": {
        "type": "object",
        "properties": {
          "url": { "type": "string" },
          "timeout": { "type": "integer" },
          "waitUntil": {
            "type": "string",
            "enum": ["load", "domcontentloaded", "networkalmostidle", "networkidle", "done"]
          }
        },
        "required": ["url"]
      }
    }
  ]
}
```

Populate all 31 entries from the pinned binary `tools/list`; do not synthesize fields from the old compatibility structs. Preserve each descriptor's `description` and `inputSchema`. Record/verify that pinned `0.3.7` advertises `outputSchema` on **0 of 31** descriptors. Within input schemas preserve property names, types, required arrays, enum values, and advertised defaults, including `getCookies.all=false` and `setChecked.checked=true` while `checked` remains required.

- [ ] **Step 4: Implement contract parsing and schema normalization**

Use a narrow model that preserves JSON schemas verbatim:

```rust
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LightpandaMcpContract {
    pub lightpanda_version: String,
    pub tool_count: usize,
    pub tools: Vec<NativeToolContract>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NativeToolContract {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
    #[serde(default, rename = "outputSchema")]
    pub output_schema: Option<serde_json::Value>,
}

pub const EXPECTED_NATIVE_TOOL_COUNT: usize = 31;

pub fn pinned_contract() -> Result<LightpandaMcpContract, CoreError> {
    let raw = include_str!("../../resources/lightpanda-mcp-contract-0.3.7.json");
    let contract: LightpandaMcpContract = serde_json::from_str(raw).map_err(|_| {
        CoreError::new(
            ErrorCode::BrowserRuntimeVersionMismatch,
            "bundled Lightpanda MCP contract snapshot is invalid",
        )
    })?;
    if contract.lightpanda_version != "0.3.7" || contract.tool_count != EXPECTED_NATIVE_TOOL_COUNT {
        return Err(CoreError::new(
            ErrorCode::BrowserRuntimeVersionMismatch,
            "bundled Lightpanda MCP contract metadata does not match the pinned runtime",
        ));
    }
    Ok(contract)
}
```

Normalize only semantically irrelevant JSON object key ordering before comparing discovered schemas. Do not discard `required`, `enum`, or `default`.

- [ ] **Step 5: Add the exact public mapping constant**

The mapping must contain exactly the 31 pairs from the spec, including camelCase-to-snake-case public naming transformations such as:

```rust
pub const NATIVE_TOOL_MAPPINGS: &[(&str, &str)] = &[
    ("goto", "browser_lightpanda_goto"),
    ("search", "browser_lightpanda_search"),
    ("markdown", "browser_lightpanda_markdown"),
    ("html", "browser_lightpanda_html"),
    ("links", "browser_lightpanda_links"),
    ("evaluate", "browser_lightpanda_evaluate"),
    ("extract", "browser_lightpanda_extract"),
    ("tree", "browser_lightpanda_tree"),
    ("nodeDetails", "browser_lightpanda_node_details"),
    ("interactiveElements", "browser_lightpanda_interactive_elements"),
    ("structuredData", "browser_lightpanda_structured_data"),
    ("detectForms", "browser_lightpanda_detect_forms"),
    ("click", "browser_lightpanda_click"),
    ("fill", "browser_lightpanda_fill"),
    ("scroll", "browser_lightpanda_scroll"),
    ("waitForSelector", "browser_lightpanda_wait_for_selector"),
    ("waitForScript", "browser_lightpanda_wait_for_script"),
    ("waitForState", "browser_lightpanda_wait_for_state"),
    ("hover", "browser_lightpanda_hover"),
    ("press", "browser_lightpanda_press"),
    ("selectOption", "browser_lightpanda_select_option"),
    ("setChecked", "browser_lightpanda_set_checked"),
    ("findElement", "browser_lightpanda_find_element"),
    ("consoleLogs", "browser_lightpanda_console_logs"),
    ("getUrl", "browser_lightpanda_get_url"),
    ("getCookies", "browser_lightpanda_get_cookies"),
    ("getEnv", "browser_lightpanda_get_env"),
    ("save", "browser_lightpanda_save"),
    ("session_new", "browser_lightpanda_session_new"),
    ("session_list", "browser_lightpanda_session_list"),
    ("session_close", "browser_lightpanda_session_close"),
];
```

The tests above enforce count and uniqueness so a missing, duplicate, or renamed pair fails immediately.

- [ ] **Step 6: Add static repository verification**

Create `scripts/verify-lightpanda-mcp-contract.py` that reads the JSON snapshot, asserts `lightpanda_version == "0.3.7"`, `tool_count == 31`, exact expected name set, unique names, and rejects any property named `session_id` in an upstream input schema because that would conflict with Local Workspaces routing metadata.

Add to `package.json`:

```json
"verify:lightpanda-contract": "python scripts/verify-lightpanda-mcp-contract.py"
```

- [ ] **Step 7: Run targeted verification**

Run:

```text
cargo test --manifest-path src-tauri/Cargo.toml browser::contract::tests -- --nocapture
npm run verify:lightpanda-contract
```

Expected: both exit 0; verifier prints Lightpanda 0.3.7, 31 expected tools, and no `session_id` collision.

- [ ] **Step 8: Commit the contract gate**

```bash
git add src-tauri/resources/lightpanda-mcp-contract-0.3.7.json src-tauri/src/browser/contract.rs src-tauri/src/browser/mod.rs scripts/verify-lightpanda-mcp-contract.py package.json
git commit -m "test(browser): freeze Lightpanda MCP contract"
```

---

### Task 2: Build an app-owned Lightpanda HTTP JSON-RPC transport with explicit session routing

**Files:**
- Create: `src-tauri/src/browser/http.rs`
- Modify: `src-tauri/src/browser/mod.rs`
- Test: `src-tauri/src/browser/http.rs`

**Interfaces:**
- Produces: `pub struct LightpandaHttpClient`
- Produces: `pub enum UpstreamToolResponse { Result(CallToolResult), JsonRpcError { code: i64, message: String, data: Option<Value> } }`
- Produces: `pub async fn initialize_default(&self) -> Result<InitializeMetadata, CoreError>`
- Produces: `pub async fn notify_initialized_default(&self) -> Result<(), CoreError>`
- Produces: `pub async fn list_tools_default(&self) -> Result<Vec<rmcp::model::Tool>, CoreError>`
- Produces: `pub async fn call_tool(&self, tool: &str, arguments: Map<String, Value>, session_id: Option<&str>) -> Result<UpstreamToolResponse, CoreError>`
- Consumed by Tasks 3 and 4.

- [ ] **Step 1: Write failing tests for the default bootstrap header and named session routing**

Use an in-process Axum fake `/mcp` endpoint that records headers and JSON-RPC bodies. The first tests must assert:

```rust
assert_eq!(seen[0].method, "initialize");
assert_eq!(seen[0].session_id.as_deref(), Some("default"));
assert_eq!(seen[1].method, "notifications/initialized");
assert_eq!(seen[1].session_id.as_deref(), Some("default"));
assert_eq!(seen[2].method, "tools/list");
assert_eq!(seen[2].session_id.as_deref(), Some("default"));

client.call_tool("getUrl", Map::new(), None).await.unwrap();
assert_eq!(last_seen_session(), Some("default"));
client.call_tool("getUrl", Map::new(), Some("alpha")).await.unwrap();
assert_eq!(last_seen_session(), Some("alpha"));
```

- [ ] **Step 2: Write failing validation tests for header injection**

Cover empty, over-256-byte, CR, LF, NUL, and other ASCII control characters:

```rust
for bad in ["", "a\rb", "a\nb", "a\0b", "a\x1fb"] {
    let err = validate_session_id(bad).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}
assert!(validate_session_id("team-a").is_ok());
assert!(validate_session_id(&"x".repeat(256)).is_ok());
assert!(validate_session_id(&"x".repeat(257)).is_err());
```

- [ ] **Step 3: Write failing response-framing and error tests**

The fake server must cover:

```text
application/json success
text/event-stream with a data: JSON-RPC response event
request-id mismatch
malformed JSON
HTTP timeout
connection loss
CallToolResult { isError:true }
JSON-RPC error { code:-32604, message:"FrameNotLoaded..." }
```

Assert a valid JSON-RPC error becomes `UpstreamToolResponse::JsonRpcError` rather than `CoreError::BrowserProtocolError`.

- [ ] **Step 4: Run the transport tests and confirm red state**

```text
cargo test --manifest-path src-tauri/Cargo.toml browser::http::tests -- --nocapture
```

Expected: compile/test failure because `LightpandaHttpClient` does not exist.

- [ ] **Step 5: Implement the client without `rmcp::StreamableHttpClientTransport`**

Use a dedicated `reqwest::Client` because Local Workspaces, not a generic MCP client transport, must select the native browsing session per request:

```rust
pub struct LightpandaHttpClient {
    client: reqwest::Client,
    endpoint: url::Url,
    next_id: std::sync::atomic::AtomicU64,
    request_timeout: Duration,
}

fn effective_session_id(session_id: Option<&str>) -> Result<&str, CoreError> {
    match session_id {
        None => Ok("default"),
        Some(id) => {
            validate_session_id(id)?;
            Ok(id)
        }
    }
}
```

Build the client with redirects disabled, bounded connect/request timeout, and a loopback endpoint only.

- [ ] **Step 6: Implement strict request/response handling**

Every request must set:

```text
Content-Type: application/json
Accept: application/json, text/event-stream
Mcp-Session-Id: <effective session>
```

Maintain monotonically increasing JSON-RPC IDs for request/response methods; notifications have no ID. Treat ID mismatch, invalid envelope, invalid SSE framing, and malformed JSON as `BROWSER_PROTOCOL_ERROR`. Never retry automatically.

- [ ] **Step 7: Preserve native tool-level errors**

For `tools/call`:

```rust
pub enum UpstreamToolResponse {
    Result(CallToolResult),
    JsonRpcError {
        code: i64,
        message: String,
        data: Option<Value>,
    },
}
```

Deserialize a successful `result` to `CallToolResult` without compatibility reshaping. Keep valid native JSON-RPC `error.code/message/data` separate from malformed protocol errors.

- [ ] **Step 8: Run the complete transport test module**

```text
cargo test --manifest-path src-tauri/Cargo.toml browser::http::tests -- --nocapture
```

Expected: all default-header, named-session, SSE, timeout, error-preservation, and no-replay tests pass.

- [ ] **Step 9: Commit the transport**

```bash
git add src-tauri/src/browser/http.rs src-tauri/src/browser/mod.rs
git commit -m "feat(browser): add native Lightpanda HTTP transport"
```

---

### Task 3: Convert runtime verification and WSL launch primitives from stdio/page mode to private HTTP MCP

**Files:**
- Modify: `src-tauri/src/browser/wsl.rs`
- Modify: `src-tauri/src/browser/runtime.rs`
- Modify: `src-tauri/src/browser/mod.rs`
- Test: `src-tauri/src/browser/wsl.rs`
- Test: `src-tauri/src/browser/runtime.rs`

**Interfaces:**
- Produces: `WslHostAdapter::spawn_lightpanda_http_server(distro_name, port) -> Result<std::process::Child, CoreError>`
- Produces: fixed PandaScript working directory `/var/lib/local-workspaces/browser/pandascripts`.
- Produces: runtime HTTP contract verifier using `LightpandaHttpClient` + `contract::verify_discovered_tools`.
- Preserves: `BrowserRuntimeManager::ensure_runtime`, `repair_runtime`, `reset_browser_data`, `remove_runtime`, `manifest`, `distro_identity`.
- Consumed by Task 4.

- [ ] **Step 1: Write a failing WSL command-shape test for the HTTP server**

The expected app-owned launch must include WSL `--cd` before `--` and Lightpanda private HTTP flags after `mcp`:

```rust
assert_eq!(args, vec![
    "-d", "LocalWorkspaces-Lightpanda",
    "-u", "localworkspaces",
    "--cd", "/var/lib/local-workspaces/browser/pandascripts",
    "--",
    "/opt/local-workspaces/lightpanda/current/lightpanda",
    "mcp",
    "--log-level", "warn",
    "--host", "127.0.0.1",
    "--port", "38741",
    "--cookie", "/var/lib/local-workspaces/browser/cookies/canonical.json",
    "--cookie-jar", "/var/lib/local-workspaces/browser/cookies/default.exit.json",
    "--http-cache-dir", "/var/lib/local-workspaces/browser/cache",
    "--http-cache-entry-limit", "1000",
]);
```

Also assert `LIGHTPANDA_DISABLE_TELEMETRY=true` and `LIGHTPANDA_DISABLE_CORE_DUMP=1` remain set.

- [ ] **Step 2: Write failing runtime-directory/reset tests for PandaScripts**

Assert provisioning creates:

```text
/var/lib/local-workspaces/browser/pandascripts
```

and Reset Browser Data removes/recreates only profile/cookies/cache/artifacts, never `pandascripts`.

- [ ] **Step 3: Replace restricted-tool smoke tests with full-contract HTTP verification tests**

The test verifier must reject:

```text
30 tools (missing one)
32 tools (unexpected one)
31 names with one schema drift
```

and accept only exact 31-tool equality. Add a regression assertion that `initialize.serverInfo.version = "0.1.0"` does not fail a runtime whose manifest/binary version is `0.3.7`.

- [ ] **Step 4: Run runtime/WSL tests and confirm failures**

```text
cargo test --manifest-path src-tauri/Cargo.toml browser::wsl::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml browser::runtime::tests -- --nocapture
```

Expected: failures referencing old stdio command shape/restricted 26-tool smoke verifier.

- [ ] **Step 5: Add fixed HTTP launch helpers**

Replace the page-oriented builder with `build_lightpanda_http_command(wsl_exe: &Path, distro_name: &str, port: u16) -> Result<std::process::Command, CoreError>`. It must set the fixed WSL working directory, fixed Lightpanda HTTP/cache/cookie argv, telemetry/core-dump environment hardening, and only format the app-selected `u16` port as a decimal argv element. Runtime verification and the production native service both use this exact builder; no stdio/page launch builder remains after Task 8.

- [ ] **Step 6: Update runtime directory fragments**

Change the provisioning fragment to create and chown:

```text
profile
cookies
cache
artifacts
pandascripts
```

Keep Reset Browser Data intentionally excluding `pandascripts` from `rm -rf`.

- [ ] **Step 7: Replace `REQUIRED_NATIVE_TOOLS` and stdio `RealMcpSmokeVerifier`**

Use the Task 1 contract resource as the only native-tool authority. Rename `McpSmokeVerifier` to `RuntimeContractVerifier` and `RealMcpSmokeVerifier` to `RealRuntimeContractVerifier` with this exact interface:

```rust
#[async_trait]
pub trait RuntimeContractVerifier: Send + Sync {
    async fn verify(&self, distro_name: &str) -> Result<(), CoreError>;
}
```

The production verifier starts a temporary private HTTP Lightpanda server on an app-selected port, verifies Windows loopback reachability, initializes on `default`, verifies no bootstrap-generated session pollution, compares all 31 tools/schemas, then terminates its own child. It must not use `initialize.serverInfo.version` for the binary version gate.

- [ ] **Step 8: Preserve the existing binary integrity/version gate**

Keep `ensure_lightpanda` verifying:

```text
SHA-256 == manifest.lightpanda_sha256
lightpanda version == manifest.lightpanda_version == 0.3.7
```

Do not weaken the existing repair behavior or WSL ownership checks.

- [ ] **Step 9: Run targeted runtime verification**

```text
cargo test --manifest-path src-tauri/Cargo.toml browser::wsl::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml browser::runtime::tests -- --nocapture
npm run verify:lightpanda-runtime
npm run verify:lightpanda-contract
```

Expected: exit 0.

- [ ] **Step 10: Commit runtime transport prerequisites**

```bash
git add src-tauri/src/browser/wsl.rs src-tauri/src/browser/runtime.rs src-tauri/src/browser/mod.rs
git commit -m "refactor(browser): provision Lightpanda HTTP MCP runtime"
```

---

### Task 4: Replace the per-page Lightpanda driver with one lazy native HTTP service

**Files:**
- Rewrite: `src-tauri/src/browser/lightpanda.rs`
- Modify: `src-tauri/src/browser/types.rs`
- Test: `src-tauri/src/browser/lightpanda.rs`

**Interfaces:**
- Produces: `pub enum UpstreamToolResponse` from Task 2 as the native outcome.
- Produces: `#[async_trait] pub trait BrowserNativeCaller: Send + Sync` for MCP-server test doubles.
- Produces: `pub struct LightpandaNativeService` for desktop lifecycle/status.
- Produces: `pub struct NativeServiceStatus { enabled, running, contract_verified, native_tool_count }`.
- Produces: `LightpandaNativeService::{new, apply_settings, status, stop, on_pause_changed}`.
- Consumed by Tasks 6 and 7.

- [ ] **Step 1: Write failing service tests for lazy single-process startup**

Use fake runtime/launcher/HTTP seams to assert:

```rust
let service = service_fixture(enabled=true);
assert!(!service.status().await.running);
service.call_native("getUrl", Map::new(), None).await.unwrap();
assert!(service.status().await.running);
assert_eq!(launch_count(), 1);
service.call_native("getUrl", Map::new(), None).await.unwrap();
assert_eq!(launch_count(), 1);
```

No public `open`/`close`/tab concept should appear in this service.

- [ ] **Step 2: Write failing lifecycle tests**

Cover:

```text
enabled=false -> BROWSER_AUTOMATION_DISABLED without runtime launch
on_pause_changed(true) -> running server stops
apply_settings(enabled=false) -> running server stops
process/HTTP loss -> current call returns process/transport error, service becomes unhealthy
next independent call may start a fresh server
failed call is never automatically replayed
```

- [ ] **Step 3: Write failing session-lock tests**

Same native session must serialize mutations; different sessions may run concurrently. Use weak keyed locks so arbitrary failed session IDs do not permanently grow the map:

```rust
fn lock_for_session(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = self.session_locks.lock().unwrap();
    map.retain(|_, weak| weak.strong_count() > 0);
    if let Some(lock) = map.get(id).and_then(std::sync::Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    map.insert(id.to_string(), Arc::downgrade(&lock));
    lock
}
```

Session-control calls use a separate global async mutex and always route over `default`.

- [ ] **Step 4: Run the Lightpanda service tests and confirm they fail against the old page driver**

```text
cargo test --manifest-path src-tauri/Cargo.toml browser::lightpanda::tests -- --nocapture
```

Expected: compile failures for missing `LightpandaNativeService`/`BrowserNativeCaller`.

- [ ] **Step 5: Define the server-facing native caller seam**

Use the exact native argument/result seam:

```rust
#[async_trait::async_trait]
pub trait BrowserNativeCaller: Send + Sync {
    async fn call_native(
        &self,
        tool: &'static str,
        arguments: serde_json::Map<String, serde_json::Value>,
        session_id: Option<&str>,
    ) -> Result<crate::browser::http::UpstreamToolResponse, CoreError>;
}
```

The trait intentionally does not expose lifecycle methods to MCP handlers.

- [ ] **Step 6: Implement `LightpandaNativeService` lifecycle**

The concrete service owns:

```text
BrowserRuntimeManager
BrowserRuntimeSettings/enabled state
one running WSL HTTP child + JobHandle
LightpandaHttpClient
contract_verified + native_tool_count
single-flight startup mutex
per-session weak keyed locks
session-control mutex
```

`ensure_running` performs the runtime gate, selects a port, starts the server, waits for Windows loopback reachability, initializes on `default`, verifies tools/schemas, verifies `session_list` did not gain a hidden bootstrap session, then publishes the running state atomically.

- [ ] **Step 7: Implement stop behavior with a bounded graceful attempt**

Add/use an app-owned fixed WSL signal path (`pkill -TERM -x lightpanda` is available in the managed distro and contains no model/user input), wait a short fixed deadline for the managed `wsl.exe` child to exit, then fall back to the existing JobHandle/process kill containment. Never kill or unregister a foreign distro; ownership remains checked by runtime management.

- [ ] **Step 8: Preserve native errors/results and no replay**

`call_native` must return `UpstreamToolResponse::Result` or `JsonRpcError` unchanged from Task 2. Connection loss marks the running instance unhealthy and returns immediately. Do not inspect native error text to translate it into Playwright-era stale-ref/target-not-found errors.

- [ ] **Step 9: Shrink browser runtime types**

Keep only types needed by settings/lifecycle, including:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserRuntimeSettings {
    pub enabled: bool,
}
```

Remove page/tab/emulation/download/image compatibility types only after their callers are migrated in later tasks; if a temporary compile dependency remains, defer that specific deletion to Task 8.

- [ ] **Step 10: Run service tests**

```text
cargo test --manifest-path src-tauri/Cargo.toml browser::lightpanda::tests -- --nocapture
```

Expected: lazy startup, single process, stop/disable/Pause, session serialization, process loss, and no-replay tests all pass.

- [ ] **Step 11: Commit the native service**

```bash
git add src-tauri/src/browser/lightpanda.rs src-tauri/src/browser/types.rs
git commit -m "feat(browser): add single native Lightpanda service"
```

---

### Task 5: Replace compatibility argument models with exact native 31-tool public schemas

**Files:**
- Rewrite: `src-tauri/src/browser/mcp.rs`
- Modify: `src-tauri/src/browser/policy.rs`
- Test: `src-tauri/src/browser/mcp.rs`

**Interfaces:**
- Produces: one `serde::Deserialize + serde::Serialize + schemars::JsonSchema` argument type per public native tool shape.
- Produces: `pub fn split_session_route<T: Serialize>(args: &T) -> Result<(Option<String>, Map<String, Value>), CoreError>`.
- Produces: `pub fn validate_native_navigation(tool: &str, args: &Map<String, Value>) -> Result<(), CoreError>`.
- Produces: `pub fn safe_native_audit_detail(tool: &str, args: &Map<String, Value>, session_id: Option<&str>) -> Value`.
- Consumed by Task 6.

- [ ] **Step 1: Write schema parity tests before replacing the structs**

For every public argument type, generate its JSON schema, remove the Local Workspaces-only `session_id` property, and compare the remaining object to the corresponding pinned upstream `inputSchema` from Task 1. The test must compare property names/types, `required`, enum arrays, and defaults.

Add this explicit collision check:

```rust
for tool in pinned_contract().unwrap().tools {
    assert!(
        tool.input_schema.pointer("/properties/session_id").is_none(),
        "upstream tool {} now owns reserved session_id",
        tool.name
    );
}
```

- [ ] **Step 2: Run parity tests and confirm the legacy structs fail**

```text
cargo test --manifest-path src-tauri/Cargo.toml browser::mcp::schema_tests -- --nocapture
```

Expected: many mismatches because current structs use `ref`, `text`, `expression`, object extraction schemas, snake_case wait models, and compatibility defaults.

- [ ] **Step 3: Define the native routing field consistently**

For non-session-control tools expose:

```rust
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct NativeRoute {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}
```

Use `#[serde(flatten)]` on public structs or explicitly repeat `session_id` if flattening produces cleaner `schemars` output. The resulting public schema must contain `session_id` as an optional top-level string.

- [ ] **Step 4: Implement exact native field names and required sets**

Representative structs must look like:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LightpandaGotoArgs {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, rename = "waitUntil", skip_serializing_if = "Option::is_none")]
    pub wait_until: Option<LightpandaLoadState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LightpandaFillArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, rename = "backendNodeId", skip_serializing_if = "Option::is_none")]
    pub backend_node_id: Option<i64>,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LightpandaExtractArgs {
    pub schema: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}
```

Implement all 31 tool argument shapes from spec Section 10; do not retain compatibility-only fields such as `ref`, `submit`, `slowly`, `values: Vec<String>`, `expression`, `max_bytes`, `timeout_ms`, browser trace actions, or run-step models.

- [ ] **Step 5: Preserve the unusual upstream `setChecked.checked` contract exactly**

The pinned schema advertises `checked` as required and also advertises default `true`. Use schemars validation metadata so public discovery keeps both properties; for example:

```rust
fn default_checked_true() -> bool { true }

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LightpandaSetCheckedArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, rename = "backendNodeId", skip_serializing_if = "Option::is_none")]
    pub backend_node_id: Option<i64>,
    #[serde(default = "default_checked_true")]
    #[schemars(required)]
    pub checked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}
```

If schemars 1.x emits a different shape, implement the smallest custom `JsonSchema` override needed so the schema parity test, not intuition, is authoritative.

- [ ] **Step 6: Implement routing stripping without mutating native arguments**

Serialize the typed public args, remove only `session_id`, and return the remaining map unchanged:

```rust
pub fn split_session_route<T: Serialize>(
    args: &T,
) -> Result<(Option<String>, Map<String, Value>), CoreError> {
    let mut obj = serde_json::to_value(args)
        .map_err(|_| CoreError::new(ErrorCode::InvalidArgument, "invalid browser arguments"))?
        .as_object()
        .cloned()
        .ok_or_else(|| CoreError::new(ErrorCode::InvalidArgument, "browser arguments must be an object"))?;
    let session_id = obj.remove("session_id").and_then(|v| v.as_str().map(str::to_string));
    Ok((session_id, obj))
}
```

Do not convert `backendNodeId`, `schema`, `script`, selectors, wait expressions, or native enum values.

- [ ] **Step 7: Restrict Local Workspaces URL policy to arguments that actually navigate**

Validate `url` only for native tools whose optional/required URL causes navigation:

```text
goto
markdown
html
links
evaluate
tree
interactiveElements
structuredData
detectForms
```

Do not treat `getCookies.url` as navigation. Keep current absolute HTTP/HTTPS validation and reject `file:`, `javascript:`, and other schemes before upstream I/O.

- [ ] **Step 8: Implement content-free audit metadata generation**

Audit helpers may record safe metadata such as origin, lengths, target kind, booleans, and enum values. They must not store raw search query, fill value, script, extraction schema, cookie/env result, page content, console content, save script/path, or raw session ID/name.

Use a bounded session fingerprint only when useful:

```rust
serde_json::json!({
    "session_present": session_id.is_some(),
    "session_bytes": session_id.map(str::len),
})
```

A hash may be added if correlation is needed; never store the raw value.

- [ ] **Step 9: Delete compatibility validation/ref code after the native tests compile**

Remove `is_valid_ref`, ref normalization, `page_evaluate_script`, synthetic drag/hover, performance/network compatibility scripts, compatibility extraction object validator, viewport/batch validators, and legacy output structs when no remaining caller uses them.

- [ ] **Step 10: Run schema and policy tests**

```text
cargo test --manifest-path src-tauri/Cargo.toml browser::mcp -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml browser::policy -- --nocapture
```

Expected: public schema minus `session_id` equals pinned native schema for all 31 mappings.

- [ ] **Step 11: Commit native schemas**

```bash
git add src-tauri/src/browser/mcp.rs src-tauri/src/browser/policy.rs
git commit -m "refactor(browser): mirror native Lightpanda tool schemas"
```

---

### Task 6: Replace all 39 browser MCP handlers with exactly 31 native `browser_lightpanda_*` handlers

**Files:**
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/src/browser/audit.rs`
- Test: `src-tauri/tests/browser_mcp_integration.rs`

**Interfaces:**
- Consumes: `Arc<dyn BrowserNativeCaller>` from Task 4.
- Consumes: Task 5 argument structs/routing/audit helpers.
- Produces: exact public browser discovery set of 31 tools.
- Produces: native output/error forwarding behavior used by Task 9 real-runtime tests.

- [ ] **Step 1: Rewrite the integration-test harness around a fake native caller**

Replace fake `BrowserManager`/page-driver fixtures with:

```rust
#[derive(Default)]
struct FakeNativeBrowser {
    calls: tokio::sync::Mutex<Vec<(String, Map<String, Value>, Option<String>)>>,
    next: tokio::sync::Mutex<VecDeque<Result<UpstreamToolResponse, CoreError>>>,
}

#[async_trait::async_trait]
impl BrowserNativeCaller for FakeNativeBrowser {
    async fn call_native(
        &self,
        tool: &'static str,
        arguments: Map<String, Value>,
        session_id: Option<&str>,
    ) -> Result<UpstreamToolResponse, CoreError> {
        self.calls.lock().await.push((
            tool.to_string(), arguments,
            session_id.map(str::to_string),
        ));
        self.next.lock().await.pop_front().unwrap_or_else(|| {
            Ok(UpstreamToolResponse::Result(CallToolResult::success(vec![ContentBlock::text("ok")])) )
        })
    }
}
```

- [ ] **Step 2: Write the exact discovery test before changing `server.rs`**

Call outer Local Workspaces `tools/list`, filter names beginning with `browser_`, and assert:

```rust
assert_eq!(all_tools.len(), 45, "53 current - 39 legacy browser + 31 native browser");
assert_eq!(browser_names.len(), 31);
assert!(browser_names.iter().all(|name| name.starts_with("browser_lightpanda_")));
assert_eq!(browser_names, expected_public_names_from_contract_mapping());
```

Because this migration does not add/remove unrelated MCP tools, total discovery is expected to move from 53 to exactly 45. Explicitly assert removed names such as `browser_get_status`, `browser_open`, `browser_snapshot`, `browser_tabs`, `browser_screenshot`, `browser_network`, `browser_trace`, and `browser_run_steps` are absent. For each mapped native tool, also compare the public structural input schema after removing `session_id` and compare `outputSchema` presence/value to the frozen upstream descriptor.

- [ ] **Step 3: Run the discovery test and confirm it fails with the current 39-tool surface**

```text
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration browser_tool_discovery_is_exact_native_31 -- --nocapture
```

Expected: FAIL showing legacy tools and missing `browser_lightpanda_*` tools.

- [ ] **Step 4: Change `WorkspaceServer` to depend on the caller trait**

Replace:

```rust
browser: Arc<crate::browser::manager::BrowserManager>
```

with:

```rust
browser: Arc<dyn crate::browser::lightpanda::BrowserNativeCaller>
```

and update `WorkspaceServer::new`/`serve` arguments accordingly. Keep desktop lifecycle on the concrete service in `AppState`; only the MCP server uses the trait object.

- [ ] **Step 5: Add one common native-call handler helper**

The helper must perform:

```text
Pause gate
BrowserNativeCaller call (service enforces enabled/lazy runtime)
Local audit write with safe metadata
native result forwarding
native JSON-RPC error -> isError result containing upstream_code/message
audit failure -> fail closed without replay
```

A valid upstream JSON-RPC error should produce content like:

```json
{"upstream_code":-32604,"message":"FrameNotLoaded: ..."}
```

for the authorized caller while audit stores only that a native upstream error occurred plus the numeric code.

- [ ] **Step 6: Register exactly the 31 handlers**

Implement thin `#[tool]` methods for:

```text
browser_lightpanda_goto
browser_lightpanda_search
browser_lightpanda_markdown
browser_lightpanda_html
browser_lightpanda_links
browser_lightpanda_evaluate
browser_lightpanda_extract
browser_lightpanda_tree
browser_lightpanda_node_details
browser_lightpanda_interactive_elements
browser_lightpanda_structured_data
browser_lightpanda_detect_forms
browser_lightpanda_click
browser_lightpanda_fill
browser_lightpanda_scroll
browser_lightpanda_wait_for_selector
browser_lightpanda_wait_for_script
browser_lightpanda_wait_for_state
browser_lightpanda_hover
browser_lightpanda_press
browser_lightpanda_select_option
browser_lightpanda_set_checked
browser_lightpanda_find_element
browser_lightpanda_console_logs
browser_lightpanda_get_url
browser_lightpanda_get_cookies
browser_lightpanda_get_env
browser_lightpanda_save
browser_lightpanda_session_new
browser_lightpanda_session_list
browser_lightpanda_session_close
```

Each handler serializes exactly its typed args, strips `session_id` where present, runs navigation policy when applicable, creates safe audit metadata, and calls the matching upstream tool name from Task 1. Do not attach compatibility `output_schema` declarations. For each public handler, mirror the pinned descriptor's output-schema presence: if upstream 0.3.7 omits `outputSchema`, omit `output_schema` in the outer `#[tool]`; if the frozen snapshot contains one, expose that same shape rather than a Local Workspaces wrapper result.

- [ ] **Step 7: Preserve native tool annotations conservatively**

Mark mutation-capable/navigation/script/session-create-close tools `read_only_hint=false`. `nodeDetails`, `findElement`, `getUrl`, `getCookies`, `getEnv`, `waitFor*`, and `session_list` may be read-only hints as specified; keep `consoleLogs` conservative because it may drain a buffer.

- [ ] **Step 8: Add forwarding/policy/audit tests**

At minimum verify:

```text
browser_lightpanda_fill forwards value but audit does not contain it
browser_lightpanda_evaluate forwards script unchanged but audit does not contain it
browser_lightpanda_extract forwards schema string unchanged
browser_lightpanda_get_cookies returns native cookie text but audit database does not contain it
browser_lightpanda_get_env returns native env text but audit database does not contain it
browser_lightpanda_save forwards path/script but audit contains neither raw value
browser_lightpanda_search audit does not contain query
named session_id is stripped from JSON args and reaches caller routing parameter
file:// URL is rejected before fake caller invocation
getCookies.url is not incorrectly treated as a navigation URL
upstream CallToolResult.isError remains isError
upstream JSON-RPC -32604 remains an upstream_code -32604 public error
```

- [ ] **Step 9: Run the rewritten MCP integration suite**

```text
cargo test --manifest-path src-tauri/Cargo.toml --test browser_mcp_integration -- --nocapture
```

Expected: exact 31-tool discovery and all native forwarding/policy/audit tests pass.

- [ ] **Step 10: Commit the public breaking API switch**

```bash
git add src-tauri/src/server.rs src-tauri/src/browser/audit.rs src-tauri/tests/browser_mcp_integration.rs
git commit -m "feat(mcp): expose exact Lightpanda native browser tools"
```

---

### Task 7: Wire the concrete native service into Tauri lifecycle, desktop runtime commands, and status UI

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/pages/settings-page.tsx`
- Test: `src-tauri/src/commands.rs`
- Create test: `src/pages/settings-page.test.ts`

**Interfaces:**
- AppState produces: `pub browser: Arc<browser::lightpanda::LightpandaNativeService>`.
- Desktop status adds: `transport`, `native_tool_count`, `contract_verified`, `native_http_sessions`, `full_native_tool_parity`.
- Preserves Tauri commands: `browser_runtime_status`, `setup_browser_runtime`, `repair_browser_runtime`, `reset_browser_data`, `remove_browser_runtime`.

- [ ] **Step 1: Write failing backend status serialization tests**

Target shape:

```rust
BrowserRuntimeStatus {
    enabled: true,
    running: true,
    backend: "lightpanda",
    transport: "private_http",
    runtime_state: RuntimeProvisionState::Ready,
    lightpanda_version: Some("0.3.7".into()),
    native_tool_count: 31,
    contract_verified: true,
    capabilities: BrowserRuntimeCapabilities {
        native_mcp: true,
        native_http_sessions: true,
        full_native_tool_parity: true,
        visual_rendering: false,
        screenshots: false,
    },
    ..
}
```

Assert serialized JSON contains no port, URL, session ID, cookie, PandaScript content/path, or absolute runtime path.

- [ ] **Step 2: Write lifecycle command tests**

Use a service seam/test constructor to prove:

```text
repair_browser_runtime stops service before repair_runtime
reset_browser_data stops service before reset
remove_browser_runtime stops service before unregister
browser_runtime_status never starts the service
```

- [ ] **Step 3: Replace `BrowserManager` construction in `lib.rs`**

Construct the concrete service from the already-created `BrowserRuntimeManager`, app settings, and fixed WSL/runtime dependencies. Pass `browser.clone()` to `server::serve` where it coerces to `Arc<dyn BrowserNativeCaller>`, and keep the concrete Arc in `AppState` for lifecycle/status.

On application exit call `browser.stop()` and retain the existing Tailscale shutdown sequence.

- [ ] **Step 4: Update settings application semantics**

`update_app_settings` continues to persist settings first, then calls:

```rust
state.browser.apply_settings(BrowserRuntimeSettings::from(&settings)).await?
```

Disabling Browser Automation must stop the native HTTP service; enabling is lazy and must not provision/start immediately.

- [ ] **Step 5: Update desktop repair/reset/remove sequencing**

Before mutating runtime files:

```rust
state.browser.stop().await?;
```

Then call the existing `BrowserRuntimeManager` operation. Repair does not eagerly restart; next native browser call starts a fresh verified server.

- [ ] **Step 6: Update TypeScript contracts and Settings copy**

Add status fields matching Rust. Export one pure constant from `settings-page.tsx` so copy is testable without a DOM harness:

```ts
export const BROWSER_NATIVE_CAPABILITY_COPY = [
  "Lightpanda native MCP over a private local HTTP channel",
  "31/31 native MCP tools available when the pinned runtime contract is verified",
  "Native isolated browser sessions are supported",
  "Lightpanda has no graphical rendering or browser screenshots",
] as const;
```

Render those four lines in the Browser Automation capability list. Export `RESET_DATA_BODY` and `REMOVE_RUNTIME_BODY` as well; Reset copy must state saved PandaScripts are preserved, while Remove Runtime copy must state saved PandaScripts inside the managed runtime are removed with the distro.

- [ ] **Step 7: Add the focused frontend copy regression test**

Create `src/pages/settings-page.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import {
  BROWSER_NATIVE_CAPABILITY_COPY,
  REMOVE_RUNTIME_BODY,
  RESET_DATA_BODY,
} from "./settings-page";

describe("native Lightpanda settings copy", () => {
  it("describes exact native parity and session support", () => {
    expect(BROWSER_NATIVE_CAPABILITY_COPY).toEqual([
      "Lightpanda native MCP over a private local HTTP channel",
      "31/31 native MCP tools available when the pinned runtime contract is verified",
      "Native isolated browser sessions are supported",
      "Lightpanda has no graphical rendering or browser screenshots",
    ]);
  });

  it("distinguishes reset from runtime removal for PandaScripts", () => {
    expect(RESET_DATA_BODY).toMatch(/preserv.*PandaScript/i);
    expect(REMOVE_RUNTIME_BODY).toMatch(/remov.*PandaScript/i);
  });
});
```

- [ ] **Step 8: Run backend and frontend focused tests/build**

```text
cargo test --manifest-path src-tauri/Cargo.toml commands::tests -- --nocapture
npm test
npm run build
```

Expected: Rust status/lifecycle tests pass, the new Vitest copy test passes, and TypeScript build accepts the new status shape.

- [ ] **Step 9: Commit desktop lifecycle/status integration**

```bash
git add src-tauri/src/lib.rs src-tauri/src/commands.rs src/lib/types.ts src/pages/settings-page.tsx src/pages/settings-page.test.ts
git commit -m "feat(browser): wire native runtime lifecycle and status"
```

---

### Task 8: Rewrite first-party agent guidance and remove compatibility-only browser architecture

**Files:**
- Modify: `src-tauri/src/agent_skills.rs`
- Modify: `src-tauri/src/browser/mod.rs`
- Modify: `src-tauri/src/browser/types.rs`
- Modify: `src-tauri/src/browser/policy.rs`
- Delete when unreferenced: `src-tauri/src/browser/driver.rs`
- Delete when unreferenced: `src-tauri/src/browser/manager.rs`
- Delete when unreferenced: `src-tauri/src/browser/artifacts.rs`
- Delete when unreferenced: `src-tauri/src/browser/files.rs`
- Test: existing agent-skill validation + Rust compile/tests.

**Interfaces:**
- First-party prompts mention only current public tools.
- Browser module exports only native service/transport/contract/runtime/policy/audit/types required by the final architecture.

- [ ] **Step 1: Add a regression search/test for obsolete public browser guidance**

The validation step must fail if first-party prompt text contains removed calls such as:

```text
browser_get_status
browser_open
browser_navigate
browser_snapshot
browser_tabs
browser_screenshot
browser_network
browser_performance
browser_trace
browser_set_viewport
browser_emulate_device
browser_run_steps
```

- [ ] **Step 2: Rewrite `AGENT_INSTRUCTIONS` browser workflow**

Use native guidance:

```text
1. Call the relevant browser_lightpanda_* tool directly; Browser Automation disabled/Pause/runtime errors are the authoritative blocker.
2. Use browser_lightpanda_goto for navigation and browser_lightpanda_tree / find_element / node_details for semantic evidence.
3. Use raw backendNodeId when convenient; prefer CSS selectors for actions intended to become replayable PandaScripts.
4. Use browser_lightpanda_console_logs, get_url, structured_data, detect_forms, evaluate, and native waits as relevant.
5. Use browser_lightpanda_session_new/list/close plus session_id for isolated contexts.
6. For visual evidence, use the separate Local Workspaces list_capture_targets/capture_screen tools; Lightpanda itself is non-visual.
7. Never bypass Browser Automation disabled/Pause with execution tools or another browser.
```

Rewrite `debug-web` with the same native vocabulary and remove stale-ref/viewport/network/screenshot claims that no longer exist in the native browser tool family.

- [ ] **Step 3: Search the repository for legacy browser API references**

Run targeted searches for `browser_open`, `browser_snapshot`, `BrowserManager`, `BrowserPageDriver`, `PageLaunchSpec`, `BrowserSessionStatus`, and opaque-ref helpers. Classify each remaining match as test/docs migration or dead code; no active production caller should remain after this task.

- [ ] **Step 4: Delete dead compatibility modules only after references are gone**

Remove `driver.rs` and `manager.rs`. Remove `artifacts.rs`/`files.rs` only if no unrelated Screen Capture/workspace code uses them. Shrink `types.rs`/`policy.rs` and `mod.rs` to the actual native architecture.

- [ ] **Step 5: Run prompt validation and compile tests**

```text
npm run validate:skill
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

Expected: no references to removed production types/tools and no prompt validation failures.

- [ ] **Step 6: Commit compatibility cleanup**

```bash
git add -A src-tauri/src/browser src-tauri/src/agent_skills.rs
git commit -m "refactor(browser): remove legacy browser compatibility layer"
```

---

### Task 9: Rewrite the real Windows/WSL integration suite to prove native sessions and all 31 public tools

**Files:**
- Rewrite: `src-tauri/tests/browser_runtime_integration.rs`
- Modify: `src-tauri/tests/fixtures/browser_test_app.rs`
- Test: `src-tauri/tests/browser_runtime_integration.rs`

**Interfaces:**
- Uses the production `BrowserRuntimeManager` and `LightpandaNativeService` through the actual outer Local Workspaces MCP server.
- Produces release evidence consumed by Task 10 release gate.

- [ ] **Step 1: Replace the test architecture comment/harness before changing cases**

The harness must reflect:

```text
MCP client -> WorkspaceServer -> BrowserNativeCaller
  -> one LightpandaNativeService HTTP server
  -> native Lightpanda sessions via Mcp-Session-Id
  -> managed WSL2 Lightpanda 0.3.7
```

Retain `LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS=1` fail-closed skip behavior and the existing Windows-host fixture bridge for pages loaded from inside WSL.

- [ ] **Step 2: Add the bootstrap/session regression test first**

After first native call/startup:

```text
browser_lightpanda_session_list
```

must report only `default` before the test creates named sessions. This proves initialization did not leak an internal `s1` session.

Then create `alpha` and `beta`, navigate them to different URLs using `session_id`, and assert `get_url` returns independent values.

- [ ] **Step 3: Add contract/version assertions**

Verify the runtime binary reports 0.3.7 through the existing runtime manager/in-distro `lightpanda version` path. Do not assert `initialize.serverInfo.version == 0.3.7`; add a regression comment/test that protocol serverInfo is not the binary pin.

- [ ] **Step 4: Build one deterministic fixture that exercises all native page features**

Extend `browser_test_app.rs` with:

```text
semantic headings/links
text input
select
checkbox
radio
buttons
hover target
keyboard target
scrollable target/page
dynamic delayed element
dynamic delayed script state
console.log output
JSON-LD/OpenGraph/Twitter/canonical metadata
cookie-setting response/page
evaluate/extract-friendly elements
```

Use fixed IDs/classes so selector-based replay tests are deterministic.

- [ ] **Step 5: Add an explicit 31-tool coverage table in the test source**

Maintain a set of upstream/public tools hit by the suite and assert it equals all 31 expected public names at the end. Group tests by behavior, but do not infer coverage from tool discovery alone.

Required calls:

```text
goto, search, markdown, html, links, evaluate, extract, tree, nodeDetails,
interactiveElements, structuredData, detectForms, click, fill, scroll,
waitForSelector, waitForScript, waitForState, hover, press, selectOption,
setChecked, findElement, consoleLogs, getUrl, getCookies, getEnv, save,
session_new, session_list, session_close
```

For `search`, use the native tool in a bounded smoke call and assert a valid native result/error path rather than depending on a particular search-engine ranking.

For `getEnv`, assert native behavior without requiring a secret LP_* variable to exist; list mode is sufficient.

- [ ] **Step 6: Add raw `backendNodeId` action parity tests**

Obtain IDs from `browser_lightpanda_tree` and pass them directly to the supported native operations:

```text
click
fill
hover
press
selectOption
setChecked
scroll
markdown/html/tree subtree calls
```

Assert no intermediate Local Workspaces selector-recovery call is needed and raw IDs remain present in native output.

- [ ] **Step 7: Add selector + PandaScript save test**

Repeat representative actions using CSS selectors, then call:

```text
browser_lightpanda_save {
  "path": "integration/replayable.js",
  "script": "<deterministic replayable script for the fixture>"
}
```

Assert the returned native path is under `/var/lib/local-workspaces/browser/pandascripts` and a `../escape.js` save returns native `isError` without escaping the fixed working directory. Cleanup the integration script with an app-owned test/runtime cleanup path rather than exposing arbitrary filesystem access through the browser tool.

- [ ] **Step 8: Add cookie/session isolation tests**

Set a cookie in one named session and verify `getCookies` in another native session does not contain it. Do not merge cookies in Local Workspaces to emulate old tabs.

- [ ] **Step 9: Add lifecycle tests for Pause/disable/process loss**

Verify one server process is used for multiple named sessions, Pause stops it, disabled Browser Automation prevents restart, re-enable allows lazy restart, and an induced process death does not replay the interrupted tool call.

- [ ] **Step 10: Run the real runtime suite in developer mode**

```text
cargo test --manifest-path src-tauri/Cargo.toml --test browser_runtime_integration --jobs 1 -- --nocapture
```

Expected on a configured Windows/WSL host: all native tests pass. If prerequisites are absent, developer mode may print explicit `SKIP [...]`; do not treat skips as release proof.

- [ ] **Step 11: Run the mandatory release-mode suite on the target Windows host**

PowerShell:

```powershell
$env:LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS = "1"
cargo test --manifest-path src-tauri/Cargo.toml --test browser_runtime_integration --jobs 1 -- --nocapture 2>&1 |
  Tee-Object -FilePath src-tauri/target/browser-runtime-integration-release.log
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
```

Expected: zero skips, zero failed tests, explicit 31/31 native coverage/parity evidence.

- [ ] **Step 12: Commit real-runtime coverage**

```bash
git add src-tauri/tests/browser_runtime_integration.rs src-tauri/tests/fixtures/browser_test_app.rs
git commit -m "test(browser): cover all native Lightpanda MCP tools"
```

---

### Task 10: Update release verification to fail closed on 31/31 native parity

**Files:**
- Modify: `scripts/verify-browser-release.ps1`
- Modify: `package.json`
- Test: `scripts/verify-browser-release.ps1`

**Interfaces:**
- Consumes: `src-tauri/target/browser-runtime-integration-release.log` from Task 9.
- Produces: release gate that proves 31 expected native tools, 31 public native tools, zero legacy browser tools, zero runtime-test skips.

- [ ] **Step 1: Write a failing verifier fixture/check for the new parity lines**

The release log must include equivalent evidence:

```text
Lightpanda version: 0.3.7
Native tools discovered: 31
Native tools expected: 31
Public browser_lightpanda tools: 31
Legacy browser tools discovered: 0
Contract parity: PASS
```

A log with 30 tools, one legacy tool, `Contract parity: FAIL`, or any `SKIP [` must fail verification.

- [ ] **Step 2: Remove the old `passed >= 19` compatibility criterion**

Replace it with semantic checks for the explicit parity lines plus the normal Rust `test result: ok` / zero-failure / zero-skip checks. Test count may grow without weakening the gate.

- [ ] **Step 3: Make the release gate invoke both static verifiers explicitly**

Keep the existing PowerShell call to:

```text
scripts/verify-lightpanda-runtime-manifest.ps1
```

and add a direct Python invocation of:

```text
scripts/verify-lightpanda-mcp-contract.py
```

Capture each exit code and append a release-gate error when either verifier fails. Preserve AGPL/source reference, notices, no-bundled-engine, installer-size, and SBOM gates.

- [ ] **Step 4: Set the package scripts to the exact final commands**

`package.json` must contain exactly these browser verification entries:

```json
"verify:lightpanda-runtime": "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-lightpanda-runtime-manifest.ps1",
"verify:lightpanda-contract": "python scripts/verify-lightpanda-mcp-contract.py",
"verify:browser-release": "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-browser-release.ps1"
```

- [ ] **Step 5: Run the static and release verifiers against the fresh release log**

```text
npm run verify:lightpanda-runtime
npm run verify:lightpanda-contract
npm run verify:browser-release
```

Expected: all exit 0 only when the real-runtime log proves full parity and the installer prerequisites are present. If the NSIS installer has not been built, `verify:browser-release` is expected to fail for that explicit reason until the release build step is run.

- [ ] **Step 6: Commit the release gate**

```bash
git add scripts/verify-browser-release.ps1 package.json
git commit -m "chore(browser): gate releases on native MCP parity"
```

---

### Task 11: Final compatibility-eradication review and full verification

**Files:**
- Review all files changed in Tasks 1–10.
- Modify only files required to fix findings from the checks below.

**Interfaces:**
- Produces final release-ready evidence.
- No new public APIs beyond the approved spec.

- [ ] **Step 1: Prove there is exactly one public browser tool family**

Search production source and first-party prompts for removed public names:

```text
browser_get_status
browser_open
browser_close
browser_navigate
browser_snapshot
browser_tabs
browser_screenshot
browser_downloads
browser_upload
browser_dialog
browser_set_viewport
browser_emulate_device
browser_devtools
browser_drag
browser_network
browser_performance
browser_trace
browser_run_steps
```

Expected: no active production handler/prompt references. Historical docs/spec migration tables are allowed.

- [ ] **Step 2: Prove old architecture types are gone from production**

Search for:

```text
BrowserManager
BrowserPageDriver
BrowserPageDriverFactory
PageLaunchSpec
ManagedPage
PageBinding
register_backend_node
resolve_ref
selector_for_ref
normalize_tree_text
normalize_backend_node_ids
```

Expected: no production references. Remove any dead compatibility module/import that remains.

- [ ] **Step 3: Run formatting and diff hygiene**

```text
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
git diff --check
```

If formatting fails, run `cargo fmt --manifest-path src-tauri/Cargo.toml`, inspect the diff, then re-run `--check`.

- [ ] **Step 4: Run the full Rust test suite**

```text
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: zero failures.

- [ ] **Step 5: Run frontend tests/build and skill validation**

```text
npm test
npm run validate:skill
npm run build
```

Expected: all exit 0.

- [ ] **Step 6: Run runtime/contract static verification**

```text
npm run verify:lightpanda-runtime
npm run verify:lightpanda-contract
```

Expected: both exit 0.

- [ ] **Step 7: Run the mandatory real Lightpanda WSL suite and capture release log**

PowerShell:

```powershell
$env:LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS = "1"
cargo test --manifest-path src-tauri/Cargo.toml --test browser_runtime_integration --jobs 1 -- --nocapture 2>&1 |
  Tee-Object -FilePath src-tauri/target/browser-runtime-integration-release.log
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
```

Expected: no skips/failures and explicit `Contract parity: PASS` with 31 discovered/31 expected/31 public/0 legacy.

- [ ] **Step 8: Build the release installer before invoking the full browser release gate**

```text
npm run tauri build
```

Expected: NSIS installer produced under `target/release/bundle/nsis` or `src-tauri/target/release/bundle/nsis` and remains below the existing 100 MiB gate.

- [ ] **Step 9: Run the full browser release gate**

```text
npm run verify:browser-release
```

Expected: exit 0 and a message equivalent to “browser release gate passed”.

- [ ] **Step 10: Review the final MCP discovery output directly**

Start the built/test Local Workspaces MCP server in the existing integration harness and print browser tool names. Verify:

```text
browser_lightpanda_* count = 31
legacy browser_* count = 0
```

Do not infer this from source code alone.

- [ ] **Step 11: Inspect final Git status/diff before claiming completion**

```text
git status --short
git diff --stat
git diff --check
```

Confirm only intended files changed and no generated runtime/browser data, cookies, PandaScripts from tests, local secrets, or release logs intended to remain untracked are accidentally staged.

- [ ] **Step 12: Commit final cleanup only if the previous gates are green**

```bash
git add -A
git commit -m "feat(browser): complete Lightpanda native MCP parity"
```

Do not create this final commit if any required verification remains red or skipped under release mode.

---

## Plan Self-Review Checklist

Before execution handoff, verify the plan against the spec:

- [ ] Contract snapshot and exact 31 mappings are covered by Task 1.
- [ ] Custom app-owned HTTP transport, SSE, request IDs, explicit `default`, named sessions, and native JSON-RPC error preservation are covered by Task 2.
- [ ] WSL private bind, fixed PandaScript cwd, full-contract runtime verification, binary pin independence from `serverInfo.version`, and Reset preservation are covered by Task 3.
- [ ] One lazy server, lifecycle, per-session serialization, no replay, Pause/disable stop, and process-loss behavior are covered by Task 4.
- [ ] Exact native argument schemas, raw `backendNodeId`, native evaluate/extract semantics, URL-policy exception for `getCookies.url`, and sensitive audit metadata are covered by Task 5.
- [ ] Exactly 31 public `browser_lightpanda_*` handlers and zero aliases are covered by Task 6.
- [ ] Tauri AppState, repair/reset/remove stop ordering, redacted runtime status, Settings copy, and PandaScript reset/remove UX are covered by Task 7.
- [ ] Agent prompt migration and dead compatibility architecture removal are covered by Task 8.
- [ ] Real native sessions, 31-tool smoke coverage, raw IDs, selector/PandaScript workflow, cookies, lifecycle, and bootstrap session regression are covered by Task 9.
- [ ] Release parity gates are covered by Task 10.
- [ ] Full build/test/real-runtime/release verification and direct discovery proof are covered by Task 11.
- [ ] No task requires CDP, Playwright/Puppeteer, screenshots, uploads/downloads, device emulation, proxy/network interception, or other out-of-scope platform work.
- [ ] No task reintroduces a raw arbitrary native-tool passthrough.
- [ ] No task relies on `initialize.serverInfo.version == 0.3.7`.
- [ ] No task initializes named Lightpanda browsing sessions separately after `session_new`.
- [ ] No task logs cookie/env/fill/query/script/schema/PandaScript/page/console content.
