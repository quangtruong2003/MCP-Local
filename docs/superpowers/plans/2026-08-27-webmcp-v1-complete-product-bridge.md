# WebMCP V1 Complete Product Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the V0 compatibility bridge into a complete product subsystem with a typed driver/service boundary, exact-origin cross-origin support, safe schema validation, async invocation/cancellation, deterministic tool-change tracking, declarative human-in-the-loop semantics, and desktop diagnostics while keeping the V0 gateway compatible.

**Architecture:** Introduce `WebMcpDriver` as the browser-mechanism contract and `WebMcpService` as the authoritative registry/invocation broker. `PageContextWebMcpDriver` implements the contract using app-owned page JavaScript and the managed browser. `WorkspaceServer` calls only the service. The service owns tool snapshots, generations, invocation records, change cursors, limits, timeout/cancel state, and cleanup on page/session lifecycle.

**Tech Stack:** Rust 2021, Tauri 2, rmcp 3.1.4, Tokio, serde/serde_json/schemars, `jsonschema` 0.50.1 with `default-features = false`, React 18/TypeScript, existing Playwright runtime and Chromium.

**Spec:** `docs/superpowers/specs/2026-08-27-webmcp-full-integration-design.md` sections 8, 10–17 plus all global security requirements.

**Prerequisite:** Every V0 task and V0 → V1 gate in `2026-08-27-webmcp-master-implementation-plan.md` is complete.

---

## Global Constraints

- Preserve all V0 public gateway names and response semantics.
- Cross-origin support is exact-origin only. Reject wildcard caller requests and non-secure remote origins before browser invocation. Treat loopback/localhost as locally trustworthy for deterministic development fixtures only when the browser itself permits them.
- Never override WebMCP Permissions Policy or `exposedTo`.
- V1 cancellation must use the browser WebMCP AbortSignal path; cancellation is not rollback.
- Browser navigation/page close may destroy page-owned bridge state. `WebMcpService` is the canonical source of invocation lifecycle and must reconcile these transitions.
- JSON Schema validation must never retrieve network/file references.
- Desktop UI may expose support/tool counts/invocation counts/waiting-for-user state, not arguments/output/page content.
- Dynamic MCP projection remains V2-only.

---

## Task 1: Extract the `WebMcpDriver` boundary and move V0 behavior behind `WebMcpService`

**Files:**
- Create: `src-tauri/src/browser/webmcp/driver.rs`
- Create: `src-tauri/src/browser/webmcp/service.rs`
- Modify: `src-tauri/src/browser/webmcp/page_context.rs`
- Modify: `src-tauri/src/browser/webmcp/mod.rs`
- Modify: `src-tauri/src/browser/manager.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/tests/webmcp_mcp_integration.rs`

### Step 1: Write a service contract test using a fake driver

- [ ] Create a minimal `FakeWebMcpDriver` in WebMCP unit tests. It must record requests and return typed results without knowing Playwright.

- [ ] Write failing tests proving the service can:
  - status;
  - list same-origin tools;
  - invoke V0 call;
  - keep V0 tool IDs stable for an unchanged generation;
  - clear registry/invocations on session shutdown.

### Step 2: Define the mechanism-neutral driver trait

- [ ] Add typed request/result structs and trait:

```rust
#[async_trait::async_trait]
pub trait WebMcpDriver: Send + Sync {
    async fn capabilities(&self, page: &WebMcpPageRef) -> Result<WebMcpCapabilities, CoreError>;
    async fn list_tools(
        &self,
        page: &WebMcpPageRef,
        from_origins: &[String],
    ) -> Result<Vec<DriverWebMcpTool>, CoreError>;
    async fn invoke(
        &self,
        page: &WebMcpPageRef,
        request: DriverInvokeRequest,
    ) -> Result<DriverInvokeStart, CoreError>;
    async fn invocation(
        &self,
        page: &WebMcpPageRef,
        request: DriverInvocationRequest,
    ) -> Result<DriverInvocationSnapshot, CoreError>;
    async fn changes(
        &self,
        page: &WebMcpPageRef,
        cursor: Option<&str>,
        wait_ms: Option<u64>,
    ) -> Result<DriverChangeBatch, CoreError>;
    async fn shutdown_page(&self, page: &WebMcpPageRef);
    async fn shutdown_session(&self, session_id: &str);
}
```

V1 `PageContextWebMcpDriver` may emulate change events/polling internally. V2 supplies native CDP behavior through the same trait.

- [ ] Define `WebMcpPageRef` using Local Workspaces identifiers plus current URL/origin; no Playwright object is allowed in this type.

### Step 3: Implement `PageContextWebMcpDriver`

- [ ] Move V0 status/list/invoke mechanism code out of `BrowserManager` into `PageContextWebMcpDriver`. Use one exact dependency boundary: define `WebMcpPageRuntime` and implement it for `BrowserManager`; `PageContextWebMcpDriver` owns `Arc<dyn WebMcpPageRuntime>`. Do not make the driver own `Arc<BrowserManager>` directly and do not use `Weak` back-references.

The construction is:

```text
BrowserManager owns BrowserDriver
BrowserManager implements WebMcpPageRuntime
PageContextWebMcpDriver owns Arc<dyn WebMcpPageRuntime>
WebMcpService owns Arc<dyn WebMcpDriver>
AppState owns Arc<BrowserManager> + Arc<WebMcpService>
```

Define the narrow runtime trait:

```rust
#[async_trait::async_trait]
pub trait WebMcpPageRuntime: Send + Sync {
    async fn webmcp_page_ref(&self) -> Result<WebMcpPageRef, CoreError>;
    async fn evaluate_app_owned(&self, function: String) -> Result<String, CoreError>;
    async fn refresh_pages(&self) -> Result<Vec<BrowserPageSummary>, CoreError>;
    async fn runtime_settings(&self) -> BrowserRuntimeSettings;
    fn is_paused(&self) -> bool;
}
```

`evaluate_app_owned` is an internal Rust trait method only. It routes to the existing allowlisted page-context evaluation path and is never exposed as an MCP tool or general caller API.

### Step 4: Implement `WebMcpService`

- [ ] Service state:

```rust
struct WebMcpServiceState {
    registries: HashMap<String, WebMcpRegistry>,       // page_id
    invocations: HashMap<String, WebMcpInvocation>,   // invocation_id
    changes: VecDeque<WebMcpChangeEvent>,
    next_invocation: u64,
    next_change: u64,
}
```

- [ ] Public methods:

```rust
pub async fn status(&self) -> Result<WebMcpStatus, CoreError>;
pub async fn list_tools(&self, req: WebMcpListRequest) -> Result<WebMcpToolSnapshot, CoreError>;
pub async fn call_tool(&self, req: WebMcpCallRequest) -> Result<WebMcpCallOutput, CoreError>;
pub async fn on_page_closed(&self, page_id: &str);
pub async fn on_browser_stopped(&self, session_id: Option<&str>);
```

V0 `call_tool` remains blocking and delegates through service.

### Step 5: Wire AppState/WorkspaceServer to service

- [ ] Add `Arc<WebMcpService>` to `AppState`.

- [ ] Pass it into `server::serve` and `WorkspaceServer::new`.

- [ ] V0 handlers call the service, not BrowserManager WebMCP methods.

- [ ] Keep lifecycle dependencies one-way with a broadcast channel owned by `BrowserManager`: add a bounded `tokio::sync::broadcast::Sender<BrowserLifecycleEvent>` plus `subscribe_lifecycle()`. Emit `PageClosed { page_id }`, `SessionStopped { session_id }`, and `ProcessLost { session_id }` at the authoritative manager transitions. `WebMcpService` subscribes once during AppState construction and runs one cleanup consumer task; BrowserManager never stores or calls the service. A lagged receiver triggers service-wide registry/invocation resync rather than guessing missed events.

### Step 6: Run regression tests and commit

- [ ] Run:

```powershell
cargo fmt --all
cargo test -p local-workspace-mcp-manager webmcp
cargo test -p local-workspace-mcp-manager --test webmcp_mcp_integration
cargo test -p local-workspace-mcp-manager --test webmcp_real_browser -- --nocapture
```

Expected: V0 behavior unchanged.

- [ ] Commit explicit paths with message:

```text
refactor(webmcp): introduce driver and service boundary
```

---

## Task 2: Add safe JSON Schema validation with external resolution disabled

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src-tauri/src/browser/webmcp/schema.rs`
- Modify: `src-tauri/src/browser/webmcp/mod.rs`
- Modify: `src-tauri/src/browser/webmcp/service.rs`
- Modify: `src-tauri/src/browser/webmcp/policy.rs`

### Step 1: Add failing schema-security tests first

- [ ] Tests must cover:
  - normal object schema validates valid input;
  - required/type errors return `WEBMCP_INPUT_INVALID` with bounded non-secret diagnostic paths;
  - malformed schema returns `WEBMCP_SCHEMA_INVALID`;
  - schema depth > 32 rejected;
  - total object-key count > 2048 rejected;
  - schema > 64 KiB rejected;
  - `$defs` local references work;
  - `$ref: "https://attacker.test/schema.json"` rejected before validator creation;
  - `$ref: "file:///C:/secret.txt"` rejected before validator creation;
  - relative external `$ref` such as `other.json#/x` rejected;
  - no test performs network/file I/O.

### Step 2: Add dependency with no ambient resolvers

- [ ] Add exactly:

```toml
jsonschema = { version = "0.50.1", default-features = false }
```

Do not enable `resolve-http`, `resolve-file`, or async resolver features.

### Step 3: Implement schema scanner and validator

- [ ] Implement iterative/recursive bounded traversal that counts depth/keys and inspects every `$ref`.

Allowed `$ref` forms:

```text
#
#/...
```

Reject all others with `WEBMCP_SCHEMA_INVALID`.

- [ ] Build validator with `jsonschema::validator_for(&schema)` only after preflight passes.

- [ ] Convert validation errors into bounded diagnostic objects containing only schema/instance paths and generic messages. Never echo whole input values into errors/audit.

### Step 4: Validate before every invocation

- [ ] `WebMcpService` validates current rediscovered schema against requested input immediately before driver invocation. A changed schema may therefore turn a formerly valid cached call into `WEBMCP_STALE_TOOL` or `WEBMCP_INPUT_INVALID` rather than invoking with mismatched input.

### Step 5: Run and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager webmcp::schema
cargo test -p local-workspace-mcp-manager webmcp
cargo clippy -p local-workspace-mcp-manager --all-targets -- -D warnings
```

Expected: PASS; no resolver features in dependency tree.

- [ ] Inspect:

```powershell
cargo tree -p local-workspace-mcp-manager -e features | Select-String jsonschema
```

Expected: no `resolve-http` / `resolve-file` feature lines.

- [ ] Commit with message:

```text
security(webmcp): validate schemas without external refs
```

---

## Task 3: Implement exact-origin cross-origin discovery

**Files:**
- Modify: `src-tauri/src/browser/webmcp/types.rs`
- Modify: `src-tauri/src/browser/webmcp/policy.rs`
- Modify: `src-tauri/src/browser/webmcp/page_context.rs`
- Modify: `src-tauri/src/browser/webmcp/driver.rs`
- Modify: `src-tauri/src/browser/webmcp/service.rs`
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/tests/webmcp_mcp_integration.rs`
- Modify: `src-tauri/tests/webmcp_real_browser.rs`
- Create: `src-tauri/tests/fixtures/webmcp/cross-origin-host.html`
- Create: `src-tauri/tests/fixtures/webmcp/cross-origin-provider.html`

### Step 1: Add request schema and policy tests

- [ ] Add:

```rust
pub struct WebMcpListRequest {
    #[serde(default)] pub from_origins: Vec<String>,
}
```

- [ ] Exact validator rules:
  - max 16 origins;
  - canonical origin only, no path/query/fragment/credentials;
  - no `*`;
  - HTTPS accepted;
  - `http://localhost[:port]`, `http://127.0.0.1[:port]`, and `http://[::1][:port]` accepted for local development;
  - other plain HTTP rejected;
  - deduplicate after canonicalization.

### Step 2: Extend app-owned discovery function

- [ ] Encode `fromOrigins` as data and call:

```javascript
await document.modelContext.getTools({ fromOrigins: payload.fromOrigins })
```

only when non-empty; otherwise retain plain `getTools()`.

- [ ] Never synthesize tools for denied iframes. Browser denial/omission is authoritative.

### Step 3: Map errors precisely

- [ ] Local policy rejection → `WEBMCP_CROSS_ORIGIN_DENIED` or `WEBMCP_CROSS_ORIGIN_UNSUPPORTED_IN_VERSION`.

- [ ] Browser Permissions Policy/provider exposure denial → `WEBMCP_PERMISSION_DENIED` or an empty authorized list as dictated by native API behavior; do not bypass.

### Step 4: Real-browser cross-origin fixture

- [ ] Run host/provider on distinct loopback origins (two ports). Provider page registers a tool with `exposedTo` matching host origin. Host iframe includes `allow="tools"`.

- [ ] Tests:
  - no `from_origins` → provider tool absent;
  - exact `from_origins` + iframe `allow="tools"` + matching `exposedTo` → tool present;
  - remove `allow="tools"` → absent/permission denied;
  - mismatched provider exposure → absent;
  - arbitrary third origin request rejected by Local Workspaces policy before browser call when syntactically insecure/invalid.

### Step 5: Run and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager cross_origin
cargo test -p local-workspace-mcp-manager --test webmcp_real_browser -- --nocapture
```

Expected: PASS.

- [ ] Commit message:

```text
feat(webmcp): support authorized cross-origin tools
```

---

## Task 4: Add page-owned async invocation runtime with AbortSignal cancellation

**Files:**
- Modify: `src-tauri/src/browser/webmcp/page_context.rs`
- Modify: `src-tauri/src/browser/webmcp/driver.rs`
- Modify: `src-tauri/src/browser/webmcp/types.rs`

### Step 1: Write codec/runtime tests

- [ ] Add fixed bridge-runtime script tests proving dynamic invocation IDs/tool inputs are encoded data only.

- [ ] Define driver requests:

```rust
pub enum DriverInvocationAction {
    Status,
    Wait { timeout_ms: u64 },
    Cancel,
}

pub struct DriverInvokeRequest {
    pub invocation_id: String,
    pub name: String,
    pub origin: String,
    pub input: serde_json::Value,
    pub from_origins: Vec<String>,
}
```

### Step 2: Implement page-global runtime

- [ ] The app-owned bridge uses a private symbol/key on `globalThis`:

```javascript
const key = Symbol.for("local-workspaces.webmcp.runtime.v1");
const runtime = globalThis[key] ||= { invocations: new Map(), changes: [] };
```

- [ ] `invoke`:
  - rediscover exact tool;
  - create `AbortController`;
  - set state running;
  - call `document.modelContext.executeTool(tool, JSON.stringify(input), { signal: controller.signal })`;
  - update completed/error/canceled state in `.then/.catch`;
  - never allow unhandled rejection;
  - return immediately with current state.

- [ ] `status/wait/cancel` uses the same map. Wait is bounded and polls state in the page runtime with a short interval; cancel calls `controller.abort()` then returns current state.

- [ ] Do not claim canceled means side effects were rolled back.

### Step 3: Handle runtime loss

- [ ] If the page document changes and the page-owned invocation record disappears, driver returns a typed `runtime_lost` signal. Service later maps it using observed page lifecycle rather than pretending `not found` means tool never ran.

### Step 4: Test with a long-running fixture

- [ ] Add an imperative fixture tool that resolves after 30 seconds and respects AbortSignal at WebMCP execution level if native semantics expose cancellation. Start, cancel, then assert terminal canceled state appears without waiting 30 seconds.

### Step 5: Run and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager invocation_cancel
cargo test -p local-workspace-mcp-manager --test webmcp_real_browser webmcp_cancellation -- --nocapture
```

Expected: PASS.

- [ ] Commit message:

```text
feat(webmcp): add abortable page invocation runtime
```

---

## Task 5: Implement `WebMcpService` invocation broker, limits, status/wait/cancel, and cleanup

**Files:**
- Modify: `src-tauri/src/browser/webmcp/service.rs`
- Modify: `src-tauri/src/browser/webmcp/types.rs`
- Modify: `src-tauri/src/browser/webmcp/policy.rs`
- Modify: `src-tauri/src/browser/webmcp/audit.rs`

### Step 1: Write state-machine tests first

- [ ] Cover every legal transition:

```text
pending -> running
running -> waiting_for_user
running -> completed
running -> error
running -> canceled
running -> navigation_triggered
waiting_for_user -> completed
waiting_for_user -> canceled
waiting_for_user -> error
```

Illegal terminal-to-nonterminal transitions must be ignored/rejected deterministically.

- [ ] Limits:
  - 4 active invocations/page;
  - 16/session;
  - 17th/5th rejected `WEBMCP_INVOCATION_LIMIT` before driver invocation;
  - max lifetime 120s;
  - client-requested timeout must be 1..120000.

### Step 2: Generate opaque invocation IDs

- [ ] Format:

```text
wi-<session counter>-<random/sha suffix>
```

IDs must not expose URL/tool names.

### Step 3: Add service APIs

- [ ] Implement:

```rust
pub async fn invoke(&self, req: WebMcpInvokeRequest) -> Result<WebMcpInvocationView, CoreError>;
pub async fn invocation(&self, req: WebMcpInvocationRequest) -> Result<WebMcpInvocationView, CoreError>;
```

Public args:

```rust
pub struct WebMcpInvokeRequest {
    pub tool_id: Option<String>,
    pub name: Option<String>,
    pub origin: Option<String>,
    #[serde(default)] pub input: serde_json::Value,
    #[serde(default)] pub from_origins: Vec<String>,
    #[serde(default)] pub wait: bool,
    #[serde(default)] pub timeout_ms: Option<u64>,
}

#[serde(tag = "action", rename_all = "snake_case")]
pub enum WebMcpInvocationRequest {
    Status { invocation_id: String },
    Wait { invocation_id: String, timeout_ms: Option<u64> },
    Cancel { invocation_id: String },
}
```

- [ ] `browser_webmcp_call_tool` becomes a compatibility wrapper around `invoke(wait=true)` and returns its established V0 `WebMcpCallOutput` shape.

### Step 4: Cleanup semantics

- [ ] Pause: abort every nonterminal page invocation, stop browser through existing path, mark unresolved records canceled/ambiguous as appropriate, and prevent new calls.

- [ ] Page close: cancel driver records if possible; terminalize Local records as canceled or error, never leave forever-running entries.

- [ ] Navigation: preserve a known `navigation_triggered` outcome; unknown runtime loss after possible side effects maps to error/ambiguous without replay.

- [ ] Browser process loss: mark all nonterminal invocations error with `BROWSER_PROCESS_LOST`; never retry.

### Step 5: Extend audit

- [ ] Invocation start/finish/cancel each must not produce duplicate MCP-call audit rows. The public MCP request gets one row; internal state transitions may use a separate internal source/category only if required, and must not log content.

### Step 6: Run and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager webmcp::service
cargo test -p local-workspace-mcp-manager invocation
```

Expected: PASS.

- [ ] Commit message:

```text
feat(webmcp): add invocation broker and lifecycle
```

---

## Task 6: Implement tool-change tracking and cursor API

**Files:**
- Modify: `src-tauri/src/browser/webmcp/page_context.rs`
- Modify: `src-tauri/src/browser/webmcp/driver.rs`
- Modify: `src-tauri/src/browser/webmcp/service.rs`
- Modify: `src-tauri/src/browser/webmcp/types.rs`

### Step 1: Define canonical change events and cursor

- [ ] Add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WebMcpChangeEvent {
    pub cursor: String,
    pub page_id: String,
    pub generation: String,
    pub added_tool_ids: Vec<String>,
    pub removed_tool_ids: Vec<String>,
    pub changed_tool_ids: Vec<String>,
}

pub struct WebMcpChangesRequest {
    #[serde(default)] pub cursor: Option<String>,
    #[serde(default)] pub wait_ms: Option<u64>,
}
```

Cursor format is opaque `wc-<counter>-<hash>`.

### Step 2: Add page `toolchange` generation signal

- [ ] App-owned runtime registers exactly one `document.modelContext` `toolchange` listener per document when available. It increments a local sequence only; it does not store unbounded tool metadata.

- [ ] `changes` call returns current local sequence and a fresh tool snapshot when sequence changed. If event support is absent, V1 driver may poll boundedly at 250 ms only during an explicit `wait_ms` call.

### Step 3: Diff in service

- [ ] Service compares normalized tool IDs/schemas/annotations and records add/remove/change events, capped at 512. When the queue rolls over, an old cursor yields a full-resync indicator instead of incorrect partial results.

### Step 4: Tests

- [ ] Add fixture controls that dynamically register/unregister a WebMCP tool after button/timeout.

- [ ] Test add/remove/change and long-poll timeout with zero changes.

### Step 5: Run and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager webmcp_changes
cargo test -p local-workspace-mcp-manager --test webmcp_real_browser toolchange -- --nocapture
```

Expected: PASS.

- [ ] Commit message:

```text
feat(webmcp): track dynamic tool changes
```

---

## Task 7: Implement declarative human-in-the-loop semantics

**Files:**
- Modify: `src-tauri/src/browser/webmcp/page_context.rs`
- Modify: `src-tauri/src/browser/webmcp/driver.rs`
- Modify: `src-tauri/src/browser/webmcp/service.rs`
- Modify: `src-tauri/src/browser/webmcp/types.rs`
- Modify: `src-tauri/tests/fixtures/webmcp/index.html`
- Create: `src-tauri/tests/fixtures/webmcp/manual-submit.html`
- Modify: `src-tauri/tests/webmcp_real_browser.rs`

### Step 1: Add capability probe for declarative metadata

- [ ] During discovery, preserve native annotation fields including autosubmit if surfaced by the current page API.

- [ ] `WebMcpCapabilities.declarative_autosubmit_metadata` is true only when the driver can distinguish it authoritatively. Do not infer by scraping arbitrary page forms.

### Step 2: Define V1 behavior

- [ ] Autosubmit declarative tool:
  - may execute through normal invoke flow;
  - `SubmitEvent.agentInvoked` / `respondWith` behavior remains website-native.

- [ ] Manual-submit declarative tool when native metadata says `autosubmit=false`:
  - Visible Debug: invocation may become `waiting_for_user`; do not click Submit.
  - Background: reject before starting with `WEBMCP_USER_ACTION_REQUIRED_IN_BACKGROUND`.

- [ ] If V1 driver cannot authoritatively classify autosubmit for a tool, expose `autosubmit=null`. Do not falsely advertise safe background autosubmit. A blocking call that actually waits for user must timeout to a stable user-action-required/error state without auto-submission.

### Step 3: Real fixture tests

- [ ] Autosubmit form proves successful tool execution and `respondWith` result.

- [ ] Manual form in Visible Debug reaches waiting state and only completes after test harness simulates the human submit through direct fixture interaction outside the WebMCP invocation path.

- [ ] Manual form in Background returns `WEBMCP_USER_ACTION_REQUIRED_IN_BACKGROUND` when classification is available.

- [ ] Add/remove tool while in-flight and verify current Chrome lifecycle: unregistering a registration does not incorrectly force-cancel an already-running execution.

### Step 4: Run and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager declarative
cargo test -p local-workspace-mcp-manager --test webmcp_real_browser declarative -- --nocapture
```

Expected: PASS.

- [ ] Commit message:

```text
feat(webmcp): preserve declarative user control
```

---

## Task 8: Expose V1 lifecycle/change MCP tools while preserving V0 wrapper

**Files:**
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/tests/webmcp_mcp_integration.rs`
- Modify: `src-tauri/tests/browser_mcp_integration.rs`
- Modify: `src-tauri/tests/mcp_integration.rs`

### Step 1: Add failing MCP contract tests

- [ ] Tool discovery must add exactly:

```text
browser_webmcp_invoke
browser_webmcp_invocation
browser_webmcp_changes
```

Total browser-prefixed tools become 35 if no other browser tools changed.

- [ ] Test schemas and behavior for invoke/status/wait/cancel/changes.

### Step 2: Add thin server handlers

- [ ] Handlers call `WebMcpService` only and use typed audit. Descriptions explicitly say website data/output is untrusted.

- [ ] Keep `browser_webmcp_call_tool` and implement it through service `invoke(wait=true)`.

### Step 3: Update agent instructions

- [ ] Teach agent:
  - use blocking V0 call for short simple tools;
  - use V1 invoke lifecycle for long-running/cancellable/user-waiting tools;
  - never infer cancellation rollback;
  - call changes/re-list after dynamic page updates.

### Step 4: Run and commit

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager --test webmcp_mcp_integration
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration
cargo test -p local-workspace-mcp-manager --test mcp_integration
```

Expected: PASS.

- [ ] Commit message:

```text
feat(webmcp): expose invocation and change tools
```

---

## Task 9: Add safe desktop WebMCP diagnostics and waiting-for-user UI

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/api.ts`
- Create: `src/components/features/settings/browser-automation-form.tsx`
- Create: `src/components/features/settings/webmcp-diagnostics.tsx`
- Modify: `src/pages/settings-page.tsx`
- Add/modify corresponding frontend tests

### Step 1: Define safe diagnostics backend shape

- [ ] Add a Tauri command returning only:

```rust
pub struct BrowserWebMcpDiagnostics {
    pub browser_enabled: bool,
    pub browser_running: bool,
    pub engine: BrowserEngine,
    pub run_mode: BrowserRunMode,
    pub webmcp_supported: bool,
    pub implementation: Option<WebMcpImplementation>,
    pub active_page_origin: Option<String>,
    pub tool_count: usize,
    pub active_invocation_count: usize,
    pub waiting_for_user_count: usize,
    pub last_error_code: Option<String>,
}
```

No tool names/descriptions, invocation input/output, full URL path/query, cookies, or browser content.

### Step 2: Extract Browser Automation settings card

- [ ] Current Settings page lacks a dedicated browser card despite backend browser settings. Create `browser-automation-form.tsx` for:
  - Browser Automation toggle;
  - browser engine selector;
  - Background/Visible Debug selector;
  - persistent-profile explanation;
  - WebMCP supported/unsupported diagnostic summary.

Do not let MCP `update_agent_settings` enable/disable this capability.

### Step 3: Add waiting-for-user state

- [ ] `webmcp-diagnostics.tsx` shows a non-secret status such as:

```text
WebMCP: supported
Tools available: 6
Active invocations: 1
Waiting for browser user action: 1
```

No form field values or tool output.

### Step 4: Frontend tests

- [ ] Test disabled, unsupported, supported, waiting-for-user, and error states with mocked API values.

### Step 5: Run and commit

- [ ] Run:

```powershell
npm test
npm run build
cargo test -p local-workspace-mcp-manager commands
```

Expected: PASS.

- [ ] Commit message:

```text
ui(webmcp): add browser diagnostics and user-wait state
```

---

## Task 10: Add deterministic agent-choice/eval fixtures

**Files:**
- Create: `src-tauri/tests/fixtures/webmcp/eval.html`
- Create: `docs/evals/webmcp-v1-cases.json`
- Create: `scripts/validate-webmcp-evals.py`
- Modify: `package.json`
- Modify: `src-tauri/src/agent_skills.rs`

### Step 1: Create a machine-readable eval dataset

- [ ] Include cases for:
  - same task available as WebMCP + click → expect WebMCP;
  - no WebMCP → expect snapshot fallback;
  - destructive-looking tool → require verification after action;
  - prompt injection in description/output → must not alter Local Workspaces policy;
  - stale tool → re-list;
  - manual declarative background → report user action blocker;
  - cross-origin authorized/unauthorized;
  - toolchange after SPA transition.

JSON cases contain expected strategy labels, not model secrets/API keys.

### Step 2: Add static validator

- [ ] `validate-webmcp-evals.py` checks uniqueness, required fields, expected strategy enum, and referenced gateway names. It does not call a cloud model.

- [ ] Add package script:

```json
"validate:webmcp-evals": "python scripts/validate-webmcp-evals.py"
```

### Step 3: Keep agent instructions concise

- [ ] Update only if the dataset exposes an instruction gap. Do not copy the entire spec into `AGENT_INSTRUCTIONS`.

### Step 4: Run and commit

- [ ] Run:

```powershell
npm run validate:webmcp-evals
cargo test -p local-workspace-mcp-manager --test mcp_integration server_instructions_define_the_agent_operating_loop
```

Expected: PASS.

- [ ] Commit message:

```text
test(webmcp): add v1 agent strategy evals
```

---

## Task 11: V1 full verification and evidence report

**Files:**
- Create: `docs/superpowers/reports/2026-08-27-webmcp-v1-verification.md`

### Step 1: Focused backend/real-browser verification

- [ ] Run:

```powershell
cargo test -p local-workspace-mcp-manager webmcp
cargo test -p local-workspace-mcp-manager --test webmcp_mcp_integration
cargo test -p local-workspace-mcp-manager --test webmcp_real_browser -- --nocapture
cargo test -p local-workspace-mcp-manager --test browser_mcp_integration
cargo test -p local-workspace-mcp-manager --test mcp_integration
npm run validate:webmcp-evals
```

Expected: all PASS.

### Step 2: Full repository verification

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

Expected: all PASS.

### Step 3: Report capability matrix

- [ ] Record V1 implementation truth for each browser engine:

```text
bundled Chromium: page-context WebMCP yes/no + cross-origin + cancel + change + autosubmit metadata
Chrome: same fields when available
Edge: same fields when available
Firefox: WebMCP unsupported, Playwright fallback supported
```

Do not claim a capability unless a test/probe confirms it.

### Step 4: Commit report

- [ ] Commit explicit report path with message:

```text
docs(webmcp): record v1 verification
```

V1 is complete only after the V1 → V2 gate in the master plan passes.
