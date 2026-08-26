# WebMCP V2 Native Runtime and Dynamic Projection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add authoritative Chromium CDP WebMCP support and optionally project desktop-trusted WebMCP tools as native MCP tools without changing the stable V0/V1 gateway contract or weakening Local Workspaces policy.

**Architecture:** Keep `WebMcpService` as the single canonical broker. Add `CdpWebMcpDriver` behind the existing `WebMcpDriver` trait. The current managed Playwright browser remains the browser owner. For Chromium-family engines, a typed internal native transport uses only fixed application-owned `browser_run_code_unsafe` programs to create persistent CDP sessions inside the existing Playwright MCP process, subscribe to the experimental `WebMCP` domain, queue native tool events, and perform invoke/cancel operations. No model-authored process code is ever accepted. Dynamic MCP projection uses rmcp 3.1.4 `ToolRoute::new_dyn` / `ToolRouter` and tool-list-change notifications, but only for exact origins explicitly trusted by desktop state.

**Tech Stack:** Rust 2021, Tauri 2, rmcp 3.1.4, Tokio, serde/serde_json/schemars, existing `jsonschema` validator, React/TypeScript, pinned managed Chromium, Chromium CDP `WebMCP` domain.

**Spec:** `docs/superpowers/specs/2026-08-27-webmcp-full-integration-design.md` sections 9–17 and all global security requirements.

**Prerequisite:** V1 complete and V1 → V2 gate passed.

---

## Global Constraints

- Native CDP is an implementation detail. Never expose arbitrary CDP method names/params to MCP clients.
- The only process-level unsafe-code calls allowed are fixed application-owned programs selected by a typed enum. Dynamic values must be base64-encoded data decoded by the fixed program.
- Add tests proving WebMCP native code templates contain no raw remote input and no generic “execute code” route.
- Native Chromium path is authoritative where available; page-context V1 driver remains fallback and compatibility oracle.
- Do not claim native support on Firefox.
- Do not create a second browser or profile for WebMCP.
- Projection is disabled/untrusted by default. Browser Automation ON is necessary but insufficient for projection.
- Trust is exact-origin and desktop-owned. Remote MCP calls cannot grant/persist projection trust.
- Projected tool descriptions/schemas remain untrusted. Apply strict projection-specific bounds/sanitization before inserting them into the MCP tool list.
- A projected tool route is only a thin selector; every call must re-authorize/re-discover through `WebMcpService`.
- If the MCP client did not negotiate tool-list-change notifications, keep gateway tools working and never assume the client refreshed dynamic tools.

---

## Task 1: Add a typed native-WebMCP transport to the browser driver without exposing generic process code

**Files:**
- Modify: `src-tauri/src/browser/driver.rs`
- Modify: `src-tauri/src/browser/playwright.rs`
- Create: `src-tauri/src/browser/webmcp/native_transport.rs`
- Modify: `src-tauri/src/browser/webmcp/mod.rs`
- Modify: `src-tauri/tests/browser_mcp_integration.rs`

### Step 1: Write failing trait/security tests

- [ ] Define a typed request enum in tests first:

```rust
#[derive(Debug, Clone)]
pub enum NativeWebMcpRequest {
    Enable,
    Snapshot,
    Invoke(NativeInvokeRequest),
    Invocation { invocation_id: String },
    Cancel { invocation_id: String },
    DrainEvents { after_sequence: u64 },
    Disable,
}
```

- [ ] Add a default BrowserDriver test proving non-Chromium/fake implementations return `BROWSER_FEATURE_UNSUPPORTED` for native WebMCP unless explicitly implemented.

- [ ] Add Playwright security tests proving:
  - no `NativeWebMcpRequest` variant accepts arbitrary JavaScript;
  - rendered process program does not contain raw tool name, origin, or input strings;
  - the only dynamic substring inserted into a program is validated standard-base64 data;
  - `GENERIC_TOOL_ALLOWLIST` still excludes `browser_run_code_unsafe`.

### Step 2: Extend BrowserDriver with one typed method

- [ ] Add:

```rust
async fn native_webmcp(
    &self,
    _request: NativeWebMcpRequest,
) -> Result<serde_json::Value, CoreError> {
    Err(CoreError::new(
        ErrorCode::BrowserFeatureUnsupported,
        "native WebMCP transport is unavailable for this browser driver",
    ))
}
```

Do not add a public/raw `run_process_code` method.

### Step 3: Create fixed native program dispatcher

- [ ] In `playwright.rs`, add private constants only, for example:

```rust
const WEBMCP_CDP_ENABLE_CODE: &str = r#"async (page) => { /* fixed */ }"#;
const WEBMCP_CDP_SNAPSHOT_CODE: &str = r#"async (page) => { /* fixed */ }"#;
const WEBMCP_CDP_INVOKE_TEMPLATE: &str = r#"async (page) => { /* decode __PAYLOAD_B64__ */ }"#;
const WEBMCP_CDP_INVOCATION_TEMPLATE: &str = r#"async (page) => { /* decode id */ }"#;
const WEBMCP_CDP_CANCEL_TEMPLATE: &str = r#"async (page) => { /* decode id */ }"#;
const WEBMCP_CDP_DRAIN_TEMPLATE: &str = r#"async (page) => { /* decode sequence */ }"#;
const WEBMCP_CDP_DISABLE_CODE: &str = r#"async (page) => { /* fixed */ }"#;
```

- [ ] Add a private `raw_app_owned_process_call()` that can only be reached from `native_webmcp` and existing fixed Chromium performance method. It still invokes upstream `browser_run_code_unsafe`, but callers cannot supply code text.

### Step 4: Run RED→GREEN and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager native_webmcp
cargo test -p local-workspace-mcp-manager playwright::tests::generic_allowlist_rejects_process_code_tools
cargo test -p local-workspace-mcp-manager playwright::tests::fixed_expressions_have_no_interpolation_points
```

Expected: PASS after implementation.

- [ ] Commit only touched files with message:

```text
security(webmcp): add typed native browser transport
```

---

## Task 2: Build the persistent Chromium CDP WebMCP event bridge

**Files:**
- Modify: `src-tauri/src/browser/playwright.rs`
- Modify: `src-tauri/src/browser/webmcp/native_transport.rs`
- Create: `src-tauri/src/browser/webmcp/cdp.rs`
- Modify: `src-tauri/src/browser/webmcp/mod.rs`

### Step 1: Define native event model and write decoding tests

- [ ] Add typed native structs matching only the CDP fields Local Workspaces needs:

```rust
pub struct NativeWebMcpTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
    pub annotations: serde_json::Value,
    pub frame_id: String,
    pub backend_node_id: Option<u64>,
    pub stack_trace: Option<serde_json::Value>,
}

pub enum NativeWebMcpEventKind {
    ToolsAdded,
    ToolsRemoved,
    ToolInvoked,
    ToolResponded,
}

pub struct NativeWebMcpEvent {
    pub sequence: u64,
    pub target_id: String,
    pub kind: NativeWebMcpEventKind,
    pub payload: serde_json::Value,
}
```

- [ ] Test unknown CDP fields are ignored/preserved only in bounded extension data; unknown enum/annotation fields must not panic.

### Step 2: Implement persistent state inside the Playwright browser process

- [ ] `WEBMCP_CDP_ENABLE_CODE` must use the current `page.context()` and attach a private state object to the persistent Playwright BrowserContext object using a symbol:

```javascript
const KEY = Symbol.for("local-workspaces.webmcp.cdp.v2");
const context = page.context();
let state = context[KEY];
if (!state) {
  state = context[KEY] = {
    enabled: true,
    sequence: 0,
    pages: new Map(),
    events: [],
    invocations: new Map(),
  };
}
```

The code must be idempotent.

- [ ] For every `context.pages()` entry:
  - create/reuse `context.newCDPSession(p)`;
  - obtain `Target.getTargetInfo` and record target id;
  - subscribe before/around `WebMCP.enable` so initial tools are not missed;
  - listen to `WebMCP.toolsAdded`, `toolsRemoved`, `toolInvoked`, `toolResponded`;
  - append bounded normalized event objects with monotonically increasing sequence;
  - cap process-side event queue to 1024 and track dropped-before sequence for resync.

- [ ] Subscribe to `context.on('page', ...)` so new tabs get native sessions automatically. On page close/session detach, remove page state and emit a synthetic local page-removed event only inside the bridge response format; do not invent a CDP event name.

### Step 3: Implement snapshot/drain/disable programs

- [ ] Snapshot returns all tracked pages, target IDs, current known tool maps, invocation summaries, current sequence, and dropped-before sequence.

- [ ] Drain returns events after a requested sequence without deleting events needed by other Rust consumers; Rust owns consumer cursor semantics.

- [ ] Disable sends `WebMCP.disable` on all live sessions, detaches listeners/sessions where possible, clears browser-process state, and returns a bounded summary.

### Step 4: Add robust unsupported-domain mapping

- [ ] Chrome/Chromium without the domain must return `WEBMCP_UNSUPPORTED`, not raw `Protocol error (WebMCP.enable): ...`.

- [ ] Firefox must never enter this process-code path.

### Step 5: Run unit/fake-driver tests and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager cdp
cargo test -p local-workspace-mcp-manager native_webmcp
cargo clippy -p local-workspace-mcp-manager --all-targets -- -D warnings
```

Expected: PASS.

- [ ] Commit message:

```text
feat(webmcp): add native cdp event bridge
```

---

## Task 3: Prove native CDP domain against bundled Chromium before routing production calls through it

**Files:**
- Modify: `src-tauri/tests/webmcp_real_browser.rs`
- Modify: `src-tauri/tests/fixtures/webmcp/index.html`
- Create: `src-tauri/tests/fixtures/webmcp/native-events.html`

### Step 1: Write native-only real-browser tests

- [ ] Add a test that starts the actual managed Chromium, opens `native-events.html`, calls `BrowserDriver::native_webmcp(Enable)`, and proves:
  - native snapshot includes page target id;
  - initial tools arrive through `toolsAdded`/snapshot;
  - dynamic registration causes `toolsAdded` event;
  - unregister causes `toolsRemoved`;
  - a tool invocation causes `toolInvoked` and terminal `toolResponded`;
  - cancel produces native canceled status when applicable;
  - native output is preserved as untrusted data.

### Step 2: Verify current bundled runtime

- [ ] Run:

```powershell
npm run verify:browser-runtime
cargo test -p local-workspace-mcp-manager --test webmcp_real_browser native_cdp -- --nocapture
```

Expected: PASS.

- [ ] If the current staged Chromium 152 lacks the exact CDP domain despite page API support, update the pinned browser runtime as a dedicated build task before continuing. Do not weaken the test or simulate the native domain.

### Step 3: If runtime upgrade is required, pin and verify it explicitly

**Files if required by the failing native test:**
- Modify: `package.json`
- Modify: `package-lock.json`
- Modify: `scripts/build-browser-runtime.ps1`
- Modify: `scripts/verify-browser-runtime.ps1`
- Modify: `src-tauri/src/browser/playwright.rs` runtime version constant

- [ ] Choose a released `@playwright/mcp`/Playwright build whose bundled Chromium exposes the tested CDP WebMCP domain; pin exact versions, rebuild staged runtime, rerun existing browser tests and native test.

- [ ] Do not move to an unpinned `latest` dependency.

### Step 4: Commit evidence-producing change

- [ ] Commit test/runtime changes separately:

```text
test(webmcp): prove native cdp domain
```

and, only if needed:

```text
build(webmcp): pin native-webmcp chromium runtime
```

---

## Task 4: Implement `CdpWebMcpDriver` and native capability selection

**Files:**
- Modify: `src-tauri/src/browser/webmcp/cdp.rs`
- Modify: `src-tauri/src/browser/webmcp/driver.rs`
- Modify: `src-tauri/src/browser/webmcp/service.rs`
- Modify: `src-tauri/src/browser/webmcp/types.rs`
- Modify: `src-tauri/src/lib.rs`

### Step 1: Write driver parity tests using fake native transport

- [ ] The same driver contract test suite used by `PageContextWebMcpDriver` must run against `CdpWebMcpDriver` for:
  - list tools;
  - invoke;
  - status/wait/cancel;
  - changes;
  - page close/session cleanup;
  - duplicate name across frames;
  - unknown annotations.

### Step 2: Add a narrow transport trait

- [ ] `CdpWebMcpDriver` must depend on:

```rust
#[async_trait::async_trait]
pub trait NativeWebMcpTransport: Send + Sync {
    async fn native_webmcp(&self, request: NativeWebMcpRequest) -> Result<serde_json::Value, CoreError>;
}
```

Implement it through the managed BrowserManager/BrowserDriver typed call; do not give Cdp driver access to generic Playwright tools.

### Step 3: Normalize CDP tools into canonical types

- [ ] Map:
  - CDP `frameId` → canonical `frame_id`;
  - `backendNodeId` → bounded diagnostic/identity field for declarative tools;
  - CDP annotation `readOnly` → `read_only_hint`;
  - `untrustedContent` → `untrusted_content_hint`;
  - `autosubmit` → `autosubmit`;
  - unknown annotations → extensions.

- [ ] Do not expose stack trace by default. Keep only bounded internal diagnostic metadata; public list output should omit it unless a future explicit diagnostic API is approved.

### Step 4: Make service choose the best driver

- [ ] Capability selection order for Chromium:

```text
native CDP healthy -> CdpWebMcpDriver
native CDP unsupported/initialization failure -> PageContextWebMcpDriver
```

Do not silently downgrade during an in-flight native invocation. Driver choice is fixed per invocation.

- [ ] `WebMcpStatus` reports actual implementation `cdp` or `page_context`.

### Step 5: Run parity + real tests and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager webmcp_driver_parity
cargo test -p local-workspace-mcp-manager --test webmcp_real_browser native_cdp -- --nocapture
cargo test -p local-workspace-mcp-manager --test webmcp_mcp_integration
```

Expected: PASS.

- [ ] Commit message:

```text
feat(webmcp): prefer native cdp driver
```

---

## Task 5: Make page/frame identity authoritative and multi-tab registry independent

**Files:**
- Modify: `src-tauri/src/browser/manager.rs`
- Modify: `src-tauri/src/browser/webmcp/cdp.rs`
- Modify: `src-tauri/src/browser/webmcp/service.rs`
- Modify: `src-tauri/src/browser/webmcp/types.rs`
- Modify: `src-tauri/tests/webmcp_real_browser.rs`
- Create: `src-tauri/tests/fixtures/webmcp/frames.html`

### Step 1: Extend native snapshot with target IDs and page order

- [ ] Native bridge must return for every Playwright page:

```json
{
  "index": 0,
  "targetId": "...",
  "url": "...",
  "title": "..."
}
```

The index is `context.pages()` order from the same browser process that owns the Page objects.

### Step 2: Bind Local page IDs to native target IDs

- [ ] Extend `PageBinding` with optional `native_target_id` but never expose raw CDP target ID through public MCP responses unless needed as opaque diagnostic metadata. Local `page_id` remains public identity.

- [ ] Reconcile native page list by exact Playwright page index first because native snapshot originates from `context.pages()`; URL/title are consistency checks, not duplicate-tab identity.

- [ ] If Playwright MCP browser tab ordering and `context.pages()` ordering disagree in a real test, fail with `WEBMCP_PROTOCOL_ERROR`; do not guess duplicate mapping.

### Step 3: Frame-qualified tool identities

- [ ] Native tool identity becomes:

```text
session_id + page_id + target_id + frame_id + origin + name + registration_generation
```

- [ ] Same tool name in top frame and iframe is not ambiguous when native frame identity differs. Same name/origin/frame with duplicate registrations remains ambiguous unless CDP registration identity provides a unique discriminator.

### Step 4: Multi-tab tests

- [ ] Real test opens two identical URLs with same tool names, verifies independent `page_id`/target binding and that invoking a tool on one selected page does not mutate the other.

- [ ] Real frame test verifies same-origin and authorized cross-origin frame tools have correct frame IDs/origins and remain separately selectable.

### Step 5: Run and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager multi_tab
cargo test -p local-workspace-mcp-manager --test webmcp_real_browser frames -- --nocapture
```

Expected: PASS.

- [ ] Commit message:

```text
feat(webmcp): add authoritative page and frame identity
```

---

## Task 6: Add desktop-owned projection trust settings and session trust store

**Files:**
- Modify: `src-tauri/src/settings.rs`
- Modify: `src/lib/types.ts`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`
- Create: `src-tauri/src/browser/webmcp/projection.rs`
- Modify: `src-tauri/src/browser/webmcp/mod.rs`
- Modify: `src/components/features/settings/webmcp-diagnostics.tsx`
- Create: `src/components/features/settings/webmcp-projection-settings.tsx`
- Modify: `src/pages/settings-page.tsx`
- Add/modify frontend/backend settings tests

### Step 1: Bump settings schema and write migration tests first

- [ ] Increment settings schema from V5 to V6 and add persistent fields:

```rust
pub webmcp_dynamic_projection_enabled: bool, // default false
pub webmcp_projection_trusted_origins: Vec<String>, // default []
```

- [ ] Migration from V5 sets false/empty without modifying browser settings.

- [ ] Existing `update_agent_settings` must not accept or mutate these fields.

### Step 2: Implement exact-origin trust validation

- [ ] Reuse strict canonical-origin parsing. Persistent trust accepts:
  - HTTPS origins;
  - loopback/localhost HTTP origins for local development.

Reject wildcard/path/query/fragment/credentials.

- [ ] Add in-memory session trust set in projection manager. Session trust is cleared on app restart; persistent trust lives in settings.

### Step 3: Add desktop commands only

- [ ] Tauri commands may:
  - enable/disable dynamic projection;
  - add/remove persistent trusted origin;
  - trust/untrust origin for current session;
  - list trusted origins.

They are not MCP tools.

### Step 4: Add UI

- [ ] Projection UI clearly separates:
  - `Expose trusted WebMCP tools directly to connected agents` toggle, OFF by default;
  - current origin trust for session;
  - persistent trusted origins list;
  - explanation that gateway tools work without projection trust.

No one-click “trust all sites”.

### Step 5: Run and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager settings
npm test
npm run build
```

Expected: PASS.

- [ ] Commit message:

```text
feat(webmcp): add desktop projection trust policy
```

---

## Task 7: Build deterministic projection names, safe MCP Tool definitions, and dynamic ToolRoutes

**Files:**
- Modify: `src-tauri/src/browser/webmcp/projection.rs`
- Modify: `src-tauri/src/browser/webmcp/schema.rs`
- Modify: `src-tauri/src/browser/webmcp/service.rs`
- Modify: `src-tauri/src/server.rs`
- Create: `src-tauri/tests/webmcp_projection_integration.rs`

### Step 1: Write projection-name/schema tests first

- [ ] Deterministic alias algorithm:

```text
origin_alias = normalized host + optional port token, lowercase, [a-z0-9_], max 40 chars
tool_alias   = normalized tool name, lowercase, [a-z0-9_], max 48 chars
base         = webmcp.<origin_alias>.<tool_alias>
collision    = append .<8 hex chars of tool identity hash>
max full MCP name = 128 bytes
```

Examples:

```text
https://shop.example.com + add-to-cart -> webmcp.shop_example_com.add_to_cart
http://localhost:5173 + Save Item -> webmcp.localhost_5173.save_item
```

- [ ] Test Unicode/punctuation/empty aliases; empty becomes `origin`/`tool` plus deterministic hash.

- [ ] Test collisions from two names normalizing to same alias get stable suffixes.

### Step 2: Sanitize projected metadata

- [ ] Projection-specific limits are stricter than gateway display limits:

```text
projected description <= 1024 bytes
projected schema <= 64 KiB and already passed V1 schema preflight
projected title <= 256 bytes
```

- [ ] Prefix projected description with a fixed Local Workspaces trust boundary sentence, then append bounded website description as quoted/untrusted context. Do not let website text replace Local Workspaces policy language.

- [ ] Map annotation hints conservatively into MCP `ToolAnnotations`; unknown website annotations remain in Local internal extensions and are not blindly injected into MCP protocol fields.

### Step 3: Create rmcp `ToolRoute::new_dyn`

- [ ] For each trusted canonical tool snapshot create a `rmcp::model::Tool` and dynamic route:

```rust
let route = ToolRoute::new_dyn(tool_attr, move |ctx| {
    Box::pin(async move {
        // extract ctx.request.arguments only
        // dispatch through ctx.service.webmcp.projected_call(projection_key, args)
        // convert CoreError to visible CallToolResult::error
    })
});
```

Do not capture a stale executable WebMCP object. Capture only a stable `ProjectionKey` containing page/origin/name/frame/generation selectors.

- [ ] `projected_call` must:
  - check Pause;
  - check Browser Automation;
  - check projection enabled;
  - re-check origin trust;
  - verify current page/session/registration generation;
  - validate input against current schema;
  - invoke through `WebMcpService`;
  - return untrusted output as normal MCP content with an explicit fixed prefix/structured marker, never as instructions.

### Step 4: Run unit integration tests and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager --test webmcp_projection_integration projection_name
cargo test -p local-workspace-mcp-manager --test webmcp_projection_integration projected_call
```

Expected: PASS.

- [ ] Commit message:

```text
feat(webmcp): build trusted dynamic tool routes
```

---

## Task 8: Add per-MCP-session dynamic ToolRouters inside `WorkspaceServer` and send list-change notifications

**Files:**
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tests/mcp_integration.rs`
- Modify: `src-tauri/tests/webmcp_projection_integration.rs`

### Step 1: Write failing dynamic-list tests

- [ ] Through a real rmcp Streamable HTTP client/server stack assert:
  - baseline static tools list is unchanged when projection is OFF;
  - trusting a fixture origin + enabling projection adds projected tools;
  - unregister/removal removes them;
  - a client initialized with tool-list-change support receives `notifications/tools/list_changed` after add/remove;
  - a client without that support is not required to receive a notification, but an explicit later `tools/list` sees the current server-side routes and gateway tools remain usable;
  - projection changes never remove static Local Workspaces tools;
  - two independent MCP sessions get independent dynamic router instances and cannot route calls through the other session's `WorkspaceServer`.

### Step 2: Replace macro-generated tool dispatch with explicit static + dynamic `ToolRouter` dispatch

- [ ] Keep `#[tool_router]` on the existing `WorkspaceServer` tool implementation so `WorkspaceServer::tool_router()` continues generating the static Local Workspaces routes. Remove `#[tool_handler]` from the `ServerHandler` implementation in V2 and implement `list_tools`, `get_tool`, and `call_tool` explicitly.

- [ ] Add fields to each `WorkspaceServer` instance:

```rust
static_tool_router: rmcp::handler::server::router::tool::ToolRouter<WorkspaceServer>,
dynamic_tool_router: Arc<std::sync::RwLock<
    rmcp::handler::server::router::tool::ToolRouter<WorkspaceServer>
>>,
projection_registry: Arc<WebMcpProjectionRegistry>,
projection_task: Arc<ProjectionTaskGuard>,
```

Construct `static_tool_router` from `WorkspaceServer::tool_router()` and populate a fresh dynamic router from the current trusted projection snapshot in `WorkspaceServer::new`. `StreamableHttpService` must continue receiving `WorkspaceServer` because rmcp 3.1.4 requires its service type to implement `ServerHandler`; do not replace it with `rmcp::Router<WorkspaceServer>`.

- [ ] Implement `list_tools` by cloning `static_tool_router.list_all()` and extending it with `dynamic_tool_router.read().unwrap().list_all()`. Reject a projected route whose name collides with any static route before insertion.

- [ ] Implement `get_tool` by checking the static router first, then the dynamic router.

- [ ] Implement `call_tool` with `ToolCallContext::new(self, request, context)`. Select exactly one router by name, clone that router while holding the read lock, release the lock before `.await`, then call it. Unknown names return the normal MCP method-not-found/protocol error; validation/runtime failures from known tools remain visible `CallToolResult::error` values.

### Step 3: Bind rmcp's public peer notifier after initialization

- [ ] In `ServerHandler::on_initialized`, call:

```rust
self.dynamic_tool_router
    .write()
    .expect("dynamic WebMCP router lock poisoned")
    .bind_peer_notifier(&context.peer);
```

rmcp 3.1.4's public `bind_peer_notifier` causes subsequent `add_route`/`remove_route` changes to request `notifications/tools/list_changed` through the initialized peer. Notification-send failure or a client that does not accept the notification must be logged/ignored as a compatibility condition, not treated as a WebMCP invocation failure.

- [ ] Also advertise `tools.list_changed = true` in `get_info()` while preserving existing prompts/tools capabilities.

### Step 4: Subscribe each server instance to shared projection diffs

- [ ] `WebMcpProjectionRegistry` exposes `subscribe() -> tokio::sync::broadcast::Receiver<ProjectionDiff>` and `snapshot()`.

- [ ] Implement `ProjectionTaskGuard` as an `Arc`-shared RAII holder around `std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>`. Its `Drop` aborts the stored task. Because cloned `WorkspaceServer` values share the same `Arc<ProjectionTaskGuard>`, the updater is aborted only when the last server clone for that MCP session is dropped.

- [ ] During/after `on_initialized`, spawn exactly one updater and store its handle in `ProjectionTaskGuard` only if the guard is currently empty. For every diff:
  - remove named routes first;
  - build/add replacement `ToolRoute::new_dyn` entries from the new projection records;
  - never mutate `static_tool_router`;
  - on broadcast lag, discard incremental assumptions and rebuild the dynamic router from `projection_registry.snapshot()`;
  - exit when the registry closes; session teardown is enforced by `ProjectionTaskGuard::drop()` aborting the task.

- [ ] Add a unit/integration assertion that cloning `WorkspaceServer` does not create a second projection updater and dropping the final clone terminates the updater.

### Step 5: Handle clients without list-change support

- [ ] Keep the dynamic router correct server-side regardless of notification delivery. A client that does not refresh automatically can still call the permanent gateway APIs and can observe current projections on a later explicit `tools/list`.

- [ ] Do not add polling from Local Workspaces into the remote MCP client and do not force reconnects.

### Step 6: Run and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager --test webmcp_projection_integration dynamic_tool_list -- --nocapture
cargo test -p local-workspace-mcp-manager --test webmcp_projection_integration independent_mcp_sessions -- --nocapture
cargo test -p local-workspace-mcp-manager --test mcp_integration
```

Expected: PASS.

- [ ] Commit message:

```text
feat(webmcp): publish trusted tools dynamically over mcp
```

---

## Task 9: Implement projection lifecycle safety, stale-route removal, and trust revocation

**Files:**
- Modify: `src-tauri/src/browser/webmcp/projection.rs`
- Modify: `src-tauri/src/browser/webmcp/service.rs`
- Modify: `src-tauri/tests/webmcp_projection_integration.rs`
- Modify: `src-tauri/tests/webmcp_real_browser.rs`

### Step 1: Write lifecycle race tests first

- [ ] Cover:
  - tool removed between MCP tools/list and call → visible `WEBMCP_STALE_TOOL`/tool-not-found result, no side effect;
  - navigation removes all old-page projected routes;
  - closing tab removes its routes;
  - Pause removes/blocks projected calls immediately;
  - Browser Automation OFF blocks calls and projection refresh;
  - projection toggle OFF removes all dynamic routes;
  - trust revocation for one origin removes only that origin’s routes;
  - browser process loss removes all browser-derived routes;
  - same name re-register with changed schema produces a new generation and route refresh.

### Step 2: Make projection registry diff authoritative

- [ ] Registry key must include origin/page/frame/tool identity and generation. A stale `ProjectionKey` cannot silently bind to a newly registered same-name tool.

- [ ] Removal is eager on known native event; full resync occurs on event-queue overflow, browser restart, driver downgrade, or inconsistent generation.

### Step 3: Add audit metadata for projected calls

- [ ] Audit adds:

```text
projection=true
projected_name
trusted_origin
registration_generation
```

and still excludes input/output body.

### Step 4: Run and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager --test webmcp_projection_integration lifecycle
cargo test -p local-workspace-mcp-manager --test webmcp_real_browser projection -- --nocapture
```

Expected: PASS.

- [ ] Commit message:

```text
security(webmcp): make projected routes fail closed
```

---

## Task 10: Preserve future WebMCP extensions and add capability-parity tests

**Files:**
- Modify: `src-tauri/src/browser/webmcp/types.rs`
- Modify: `src-tauri/src/browser/webmcp/cdp.rs`
- Modify: `src-tauri/src/browser/webmcp/page_context.rs`
- Modify: `src-tauri/src/browser/webmcp/service.rs`
- Create: `src-tauri/tests/webmcp_capability_parity.rs`
- Modify: `docs/evals/webmcp-v1-cases.json`

### Step 1: Define bounded extension containers

- [ ] Unknown/future metadata lives only in:

```rust
pub extensions: serde_json::Map<String, serde_json::Value>
```

with total serialized size <= 64 KiB per tool/event/output metadata object.

- [ ] Known V2 annotations keep typed fields; unknown keys preserved in extensions rather than dropped.

### Step 2: Add parity matrix test

- [ ] For each canonical capability assert page-context/CDP behavior or explicit unsupported reason:

```text
same-origin discovery
cross-origin discovery
imperative tools
declarative tools
readOnly annotation
untrustedContent annotation
autosubmit annotation
invoke
cancel
change tracking
frame identity
backendNodeId
native invocation events
navigation outcome
unknown annotation preservation
```

- [ ] A native capability regression must fail the parity suite rather than silently falling back without reporting implementation downgrade.

### Step 3: Add forward-compatibility fixtures

- [ ] Feed synthetic CDP/page metadata with unknown annotation keys such as `futureConsequentialAction` and unknown extension objects; assert they survive bounded normalization and never become authorization automatically.

### Step 4: Run and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager --test webmcp_capability_parity
npm run validate:webmcp-evals
```

Expected: PASS.

- [ ] Commit message:

```text
test(webmcp): enforce native capability parity
```

---

## Task 11: Complete V2 desktop UX for native/projection status

**Files:**
- Modify: `src/components/features/settings/webmcp-diagnostics.tsx`
- Modify: `src/components/features/settings/webmcp-projection-settings.tsx`
- Modify: `src/lib/types.ts`
- Modify: `src-tauri/src/commands.rs`
- Add/modify frontend tests

### Step 1: Extend safe diagnostics

- [ ] Show:
  - implementation: Native CDP / Page Context / Unsupported;
  - active page origin;
  - native registry tool count;
  - projected tool count;
  - projection enabled/disabled;
  - current origin trusted session/persistent/neither;
  - active/waiting invocation counts;
  - last stable error code.

Do not show tool descriptions/output/arguments by default.

### Step 2: Add projection trust controls

- [ ] Session trust action affects current origin only.

- [ ] Persistent trust requires an explicit confirmation affordance in desktop UI and exact origin display.

- [ ] Revocation immediately updates projection registry/tool list.

### Step 3: Run and commit

- [ ] Run:

```powershell
npm test
npm run build
cargo test -p local-workspace-mcp-manager commands
```

Expected: PASS.

- [ ] Commit message:

```text
ui(webmcp): expose native and projection controls
```

---

## Task 12: V2 real-browser, notification, security, and full-repo verification

**Files:**
- Create: `docs/superpowers/reports/2026-08-27-webmcp-v2-verification.md`

### Step 1: Run focused WebMCP suites

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager webmcp
cargo test -p local-workspace-mcp-manager --test webmcp_mcp_integration
cargo test -p local-workspace-mcp-manager --test webmcp_real_browser -- --nocapture
cargo test -p local-workspace-mcp-manager --test webmcp_projection_integration -- --nocapture
cargo test -p local-workspace-mcp-manager --test webmcp_capability_parity
npm run validate:webmcp-evals
```

Expected: all PASS.

### Step 2: Run security regression suites

- [ ] Explicitly verify:
  - prompt-injection-looking descriptions/results do not change policy or enable projection;
  - generic `browser_run_code_unsafe` is still absent from public MCP tools;
  - dynamic payloads are never raw-interpolated into process code;
  - untrusted external `$ref` is blocked with no network/file resolver;
  - trust cannot be granted by MCP tool;
  - Pause blocks gateway and projected calls;
  - projection stale route does not replay/rebind;
  - audit omits inputs/outputs/secrets.

### Step 3: Run full repository verification

- [ ] Run:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm test
npm run build
npm run validate:skill
npm run validate:webmcp-evals
npm run verify:browser-runtime
```

If Task 3 changed browser runtime also run:

```powershell
npm run build:browser-runtime
npm run verify:browser-runtime
```

Expected: all PASS.

### Step 4: Write spec-to-evidence report

- [ ] Report must map every final success criterion from the design spec to:
  - implementation file(s);
  - test name(s);
  - runtime evidence;
  - unsupported browser behavior/fallback where applicable.

- [ ] Include exact rmcp version (`Cargo.lock`) and actual staged Chromium/browser runtime versions.

### Step 5: Commit report

- [ ] Commit:

```text
docs(webmcp): record v2 full verification
```

V2 is complete only after the master-plan V2 completion gate is satisfied.
