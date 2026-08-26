# WebMCP Full Integration Spec — Self Review

**Date:** 2026-08-27
**Spec reviewed:** `docs/superpowers/specs/2026-08-27-webmcp-full-integration-design.md`
**Review outcome:** Approved for implementation planning after the amendments recorded below.

## Review criteria

The spec was reviewed against:

1. current Chrome WebMCP page API behavior;
2. current Chromium CDP `WebMCP` domain behavior;
3. the repository's actual BrowserManager / Playwright MCP / rmcp architecture;
4. security boundaries around page data, process-level code, schema validation, cross-origin access, Pause, audit, and dynamic MCP routing;
5. whether V0, V1, and V2 can each end in a testable independent release gate;
6. whether the implementation plan can be executed without unresolved architectural choices.

## Amendments made to the spec

### 1. Normalize current `getTools()` shape explicitly

Current Chrome discovery exposes WebMCP `inputSchema` as JSON text in the page API shape rather than guaranteeing an already-parsed Rust/JSON object. The spec now requires bounded JSON parsing before canonical normalization.

The browser discovery object may also carry page/browser references such as `window`. The spec now explicitly forbids serializing or exposing raw `Window` references through MCP.

Implementation consequence: V0 page-context code converts native objects to plain data inside the page, and Rust parses/bounds only that plain data.

### 2. Make V1 cancellation concrete

The current WebMCP page API supports an AbortSignal option on tool execution. The spec now names the concrete call shape:

```javascript
document.modelContext.executeTool(tool, inputJson, {
  signal: controller.signal,
})
```

Implementation consequence: V1 can implement real cancellation in the page-context driver without waiting for V2 CDP support. Cancellation still means “stop waiting/request cancellation,” not rollback of already-applied side effects.

### 3. Make rmcp V2 projection an implementable requirement

The repository resolves `rmcp` to 3.1.4. That version already provides dynamic `ToolRouter` mutation, `ToolRoute::new_dyn`, peer notifier binding, and `notifications/tools/list_changed` support.

The spec now treats server-side dynamic projection as implementable rather than speculative. Client notification acceptance remains capability-dependent, so the permanent WebMCP gateway remains the fallback.

A planning-level correction was also made after checking the actual transport generic: `StreamableHttpService` requires a `ServerHandler`, while `rmcp::Router<S>` implements `Service<RoleServer>`. Therefore the V2 execution plan does **not** replace `WorkspaceServer` with `Router<WorkspaceServer>`. Instead, `WorkspaceServer` owns per-session static + dynamic `ToolRouter` state and explicitly implements tool listing/calling while binding the peer notifier in `on_initialized`.

### 4. Close the untrusted JSON Schema resolver gap

The spec originally required safe schema parsing but did not explicitly prohibit ambient external reference retrieval.

That is now explicit:

- no HTTP `$ref` retrieval;
- no filesystem `$ref` retrieval;
- no relative external document retrieval;
- local in-memory references such as `$defs` remain supported.

The V1 plan pins:

```toml
jsonschema = { version = "0.50.1", default-features = false }
```

and preflights every `$ref` before validator construction.

This prevents website-controlled schemas from becoming an SSRF or local-file-read primitive.

## Architecture review result

The core architecture remains sound:

```text
Remote MCP agent
  -> Local Workspaces WorkspaceServer
  -> WebMcpService
  -> WebMcpDriver
      -> PageContextWebMcpDriver (V0/V1 compatibility path)
      -> CdpWebMcpDriver (V2 native Chromium path)
  -> BrowserManager
  -> private Playwright MCP child
  -> Local Workspaces managed browser/profile
```

Local Workspaces remains the only public MCP server.

The following boundaries are retained:

- Playwright MCP remains private stdio;
- raw CDP is never exposed remotely;
- `browser_run_code_unsafe` remains absent from the public MCP tool surface;
- fixed application-owned process code is permitted only behind a typed internal native-WebMCP request enum in V2;
- remote/model input is encoded as data, never raw-interpolated into executable JavaScript;
- Browser Automation and Pause remain authoritative;
- filesystem workspace policy remains independent;
- WebMCP data never becomes trusted Local Workspaces policy.

## Implementation-plan corrections made during self review

### V1 dependency graph

The draft plan initially left two possible ownership patterns for the page-context driver. That ambiguity was removed.

The implementation plan now requires exactly:

```text
BrowserManager owns BrowserDriver
BrowserManager implements WebMcpPageRuntime
PageContextWebMcpDriver owns Arc<dyn WebMcpPageRuntime>
WebMcpService owns Arc<dyn WebMcpDriver>
AppState owns Arc<BrowserManager> + Arc<WebMcpService>
```

Browser lifecycle cleanup uses a bounded Tokio broadcast channel from BrowserManager to WebMcpService, so BrowserManager does not own/call the service and there is no reference cycle.

### V2 dynamic MCP routing

The draft plan incorrectly proposed returning `rmcp::Router<WorkspaceServer>` from the existing Streamable HTTP service factory. Current rmcp transport bounds do not allow that because the factory expects `S: ServerHandler`.

The corrected V2 plan requires each `WorkspaceServer` MCP session to own:

```text
static ToolRouter<WorkspaceServer>
dynamic ToolRouter<WorkspaceServer>
shared projection registry subscription
per-session peer notifier binding
```

`list_tools`, `get_tool`, and `call_tool` are implemented explicitly in V2. Static routes still come from the existing `#[tool_router]` generated router, so existing tool implementations are not duplicated.

A per-session RAII task guard aborts the projection updater when the final WorkspaceServer clone for that MCP session is dropped.

## Residual implementation risks and their gates

### Native CDP availability in the staged browser

The page API and the tip-of-tree CDP domain are separate surfaces. V2 therefore has a real-browser native-domain proof **before** production routing switches to CDP.

If the currently staged Chromium cannot pass that test, the implementation must pin a released Playwright/browser runtime that does. It must not fake the domain or silently mark native support true.

### Declarative autosubmit metadata on the page-context driver

V1 reports `declarative_autosubmit_metadata` only when the managed browser exposes authoritative metadata through the page API. It does not scrape arbitrary DOM forms to invent WebMCP metadata.

V2 CDP annotations become the authoritative Chromium source for `autosubmit` and declarative `backendNodeId` where available.

### Experimental API drift

WebMCP is still evolving. The canonical Rust model includes bounded extension maps, feature negotiation, implementation reporting, and explicit unsupported states.

Unknown future annotations are preserved as untrusted extension data but never gain authorization semantics automatically.

### MCP client dynamic-list behavior

Server dynamic tool routing is supported by rmcp 3.1.4, but clients may not refresh dynamically. The system therefore never removes the stable gateway APIs. A client that does not process `tools/list_changed` can continue using the gateway or refresh tools explicitly.

## Final review verdict

The spec is internally coherent after the amendments above and is suitable for implementation.

The work is intentionally decomposed into four planning artifacts:

1. `docs/superpowers/plans/2026-08-27-webmcp-master-implementation-plan.md`
2. `docs/superpowers/plans/2026-08-27-webmcp-v0-compatibility-bridge.md`
3. `docs/superpowers/plans/2026-08-27-webmcp-v1-complete-product-bridge.md`
4. `docs/superpowers/plans/2026-08-27-webmcp-v2-native-runtime-projection.md`

Each version has an independent completion gate. V1 cannot begin until V0 passes, and V2 cannot begin until V1 passes.

No implementation should begin by bypassing those release gates or by implementing V2 projection directly against the current static server router.
