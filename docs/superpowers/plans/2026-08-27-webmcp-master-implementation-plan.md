# WebMCP Full Integration — Master Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver complete, secure WebMCP consumption in Local Workspaces across V0 → V1 → V2 while keeping Local Workspaces as the only public MCP server and preserving the existing Browser Automation, Pause, audit, profile, and filesystem security boundaries.

**Architecture:** Keep the existing `WorkspaceServer → BrowserManager → private Playwright MCP → managed browser` path. V0 adds a fixed page-context WebMCP compatibility bridge. V1 extracts a typed `WebMcpDriver` + `WebMcpService` broker with cross-origin, async lifecycle, cancellation, schema validation, tool-change tracking, and declarative human-in-the-loop behavior. V2 adds native Chromium CDP WebMCP event/invocation support behind the same service and dynamically projects user-trusted WebMCP tools into the MCP tool list using rmcp 3.1.4 dynamic routing/list-change notification support.

**Tech Stack:** Rust 2021, Tauri 2, rmcp 3.1.4, Tokio, serde/serde_json, schemars, `jsonschema` 0.50.1 with default features disabled, React 18 + TypeScript/Vite, pinned `@playwright/mcp` 0.0.79 / Playwright runtime initially, bundled Chromium 152 initially, Chrome WebMCP page APIs, Chromium CDP `WebMCP` domain in V2.

**Spec:** `docs/superpowers/specs/2026-08-27-webmcp-full-integration-design.md`

---

## Global Constraints

- Local Workspaces remains the only public MCP endpoint. Never expose Playwright MCP, raw CDP, a WebMCP sidecar, or a browser debugging port through Tailscale/Funnel.
- Pause has highest priority. `ACCESS_PAUSED` must terminate/block WebMCP activity before Browser Automation, WebMCP capability, or page state is consulted.
- Browser Automation OFF blocks discovery/invocation/change reads. Only safe status reporting may remain readable without touching page contents.
- Never use model-authored JavaScript to implement WebMCP. V0/V1 page-context code is application-owned. Dynamic model/user data is encoded as data, not concatenated as executable source.
- Every WebMCP title, description, schema, annotation, error, and result is untrusted website data.
- `readOnlyHint`, `untrustedContentHint`, `autosubmit`, and future annotations are hints/metadata; they never lower Local Workspaces authorization.
- Never persist invocation inputs, outputs, form values, cookies, tokens, Authorization headers, browser storage, or raw JS stacks in audit/diagnostics.
- Never allow JSON Schema external `$ref` retrieval over network or filesystem. The validator dependency must be configured with `default-features = false`; only in-memory/local references such as `$defs` are supported.
- Cross-origin access must obey browser Permissions Policy, `allow="tools"`, provider `exposedTo`, secure-context rules, and exact requested origins. Local Workspaces must not inject/bypass any of them.
- A state-changing WebMCP invocation is never auto-replayed after timeout, browser process loss, ambiguous transport failure, or navigation.
- Existing Playwright semantic browser automation remains the permanent fallback for non-WebMCP pages/browsers.
- Preserve all unrelated working-tree changes. Before editing an existing file, read its latest revision and patch from that revision.
- The current repo has browser work in progress. Each implementation commit must contain only files intentionally touched by that task.

## Program Decomposition

| Release | Execution plan | Outcome |
|---|---|---|
| V0 | `docs/superpowers/plans/2026-08-27-webmcp-v0-compatibility-bridge.md` | Same-origin WebMCP status/list/call through fixed page-context bridge, bounded/audited, real Chromium smoke-tested |
| V1 | `docs/superpowers/plans/2026-08-27-webmcp-v1-complete-product-bridge.md` | `WebMcpService` + driver abstraction, cross-origin, schema validation, async/cancel lifecycle, declarative HITL, changes, desktop diagnostics/evals |
| V2 | `docs/superpowers/plans/2026-08-27-webmcp-v2-native-runtime-projection.md` | Native CDP registry/invoke/cancel/frame identity, multi-tab state, trusted-origin dynamic MCP projection and list-change notifications |

## Release Gates

### V0 → V1 gate

Do not begin V1 until all are true:

- `browser_webmcp_get_status`, `browser_webmcp_list_tools`, and `browser_webmcp_call_tool` are discoverable over the real rmcp server.
- Bundled Chromium launches with WebMCP enabled and a deterministic local fixture proves imperative + declarative discovery and read/write invocation.
- Prompt-injection-looking metadata/output is returned only as marked untrusted data.
- Pause and Browser Automation OFF tests prove no page-context call reaches the driver.
- Audit tests prove actual input/output are absent.
- Existing browser MCP integration tests still pass.

### V1 → V2 gate

Do not begin V2 until all are true:

- V0 gateway behavior is routed through `WebMcpService` and the `WebMcpDriver` abstraction without regressions.
- Exact-origin cross-origin behavior has deterministic allow/deny coverage.
- Async invocation, status/wait/cancel, timeout, navigation, process-loss, and Pause cleanup pass fake-driver tests.
- Page-context cancellation uses `executeTool(..., { signal })` and a real long-running fixture proves cancellation.
- JSON Schema validation runs with external resolvers disabled.
- Declarative autosubmit/manual behavior is correctly surfaced for the managed browser capability set.
- Tool-change cursor/registry behavior is deterministic.
- Desktop diagnostics/waiting-for-user UI exposes only safe metadata.

### V2 completion gate

The program is complete only when all are true:

- Native Chromium CDP `WebMCP.enable`, tools add/remove, invoke, respond, and cancel work in the managed browser.
- Page + frame identity is authoritative for native Chromium and duplicate-name tools are unambiguous.
- Multiple tabs maintain independent registries.
- V0/V1 gateway calls continue unchanged while native CDP is active.
- Dynamic projection is OFF/untrusted by default and is enabled only by desktop-owned exact-origin trust.
- Projected tools use sanitized deterministic MCP names and remain routed through `WebMcpService`.
- Projected tools are removed on unregister/navigation/close and stale calls fail safely.
- rmcp tool-list-change notifications are sent only when the connected client negotiated acceptance; gateway fallback remains usable otherwise.
- Unknown/future annotations survive in extension maps.
- Full repo verification and real-browser suite pass.

## Cross-Version Public MCP Contract

The following gateway tools become permanent compatibility APIs:

```text
browser_webmcp_get_status
browser_webmcp_list_tools
browser_webmcp_call_tool       # V0 convenience API, retained forever
browser_webmcp_invoke          # V1+
browser_webmcp_invocation      # V1+: status | wait | cancel
browser_webmcp_changes         # V1+
```

V2 may additionally expose projected tools with generated names:

```text
webmcp.<origin_alias>.<tool_alias>[.<collision_suffix>]
```

Projected calls must never bypass the gateway service path.

## Canonical Limits

Use centralized constants and tests. These are the initial product limits for the plan:

```text
MAX_WEBMCP_TOOLS_PER_PAGE              = 128
MAX_WEBMCP_TOOLS_PER_SESSION           = 512
MAX_WEBMCP_NAME_BYTES                  = 256
MAX_WEBMCP_TITLE_BYTES                 = 512
MAX_WEBMCP_DESCRIPTION_BYTES           = 8 KiB
MAX_WEBMCP_SCHEMA_BYTES                = 64 KiB per tool
MAX_WEBMCP_SCHEMA_DEPTH                = 32
MAX_WEBMCP_SCHEMA_OBJECT_KEYS          = 2048
MAX_WEBMCP_INPUT_BYTES                 = 256 KiB
MAX_WEBMCP_OUTPUT_BYTES                = 512 KiB
MAX_WEBMCP_EXTENSION_BYTES             = 64 KiB
MAX_WEBMCP_CROSS_ORIGINS               = 16
MAX_WEBMCP_ACTIVE_INVOCATIONS_SESSION  = 16
MAX_WEBMCP_ACTIVE_INVOCATIONS_PAGE     = 4
MAX_WEBMCP_CHANGE_EVENTS               = 512
MAX_WEBMCP_INVOCATION_LIFETIME_MS      = 120_000
```

If implementation evidence shows one bound is incompatible with normal WebMCP behavior, change the constant and its tests deliberately; do not silently make outputs unbounded.

## Required Verification Per Release

Run the narrow release tests first, then the full repository checks.

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm test
npm run build
npm run validate:skill
npm run verify:browser-runtime
```

For releases that change the packaged browser runtime also run:

```powershell
npm run build:browser-runtime
npm run verify:browser-runtime
```

The real-browser WebMCP suite must be run against the staged runtime, not a random system browser.

## Commit Discipline

- One logical task per commit.
- Suggested prefixes: `feat(webmcp):`, `test(webmcp):`, `refactor(webmcp):`, `security(webmcp):`, `ui(webmcp):`, `build(webmcp):`.
- Never use `git add -A` in this working tree. Stage explicit paths from the task only.
- Before every commit run `git diff --cached --check` and inspect `git diff --cached --name-only`.

## Execution Order

- [ ] Complete every task in the V0 plan and pass the V0 → V1 gate.
- [ ] Complete every task in the V1 plan and pass the V1 → V2 gate.
- [ ] Complete every task in the V2 plan and pass the V2 completion gate.
- [ ] Run full repository verification one final time.
- [ ] Produce a final implementation report mapping each spec success criterion to code/tests/evidence.
