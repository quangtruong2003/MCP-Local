# Lightpanda Native MCP Full Parity — Design Specification

**Date:** 2026-08-28  
**Status:** Approved design direction  
**Target platform:** Windows + managed WSL2 runtime  
**Pinned browser baseline:** Lightpanda `0.3.7` x86_64 Linux  
**Pinned SHA-256:** `895339b02205171a181dde743ae0068bb4564884076feac8482baca9c212aa5a`

## 1. Goal

Replace the current Local Workspaces Browser Automation compatibility layer with a single Lightpanda-native MCP surface.

The public MCP contract must expose every native tool advertised by the pinned `lightpanda mcp` runtime and must not expose a second, overlapping legacy `browser_*` tool family.

The resulting model-facing architecture is:

```text
ChatGPT / MCP client
        |
        | Local Workspaces MCP
        v
Policy + Audit + Pause + Browser Enable Gate
        |
        | browser_lightpanda_*
        v
Local Workspaces Lightpanda HTTP adapter
        |
        | private HTTP MCP
        | POST /mcp
        | optional Mcp-Session-Id
        v
Lightpanda 0.3.7 running in managed WSL2
```

The primary success criterion is measurable:

> For the pinned Lightpanda binary, every tool returned by upstream `tools/list` has exactly one public Local Workspaces `browser_lightpanda_*` counterpart, and no old public `browser_*` compatibility tool remains registered.

---

## 2. Scope

### 2.1 In scope

This specification covers **full native MCP tool parity** for the pinned Lightpanda runtime.

It includes:

- all native Lightpanda MCP tools;
- native Lightpanda HTTP MCP transport;
- native Lightpanda sessions;
- raw Lightpanda `backendNodeId` targeting;
- native tool argument semantics;
- direct upstream tool result forwarding;
- Local Workspaces policy, audit, Pause, runtime provisioning, and process ownership around the native surface;
- lazy runtime startup;
- browser runtime diagnostics in the desktop UI;
- removal of the current compatibility browser API.

### 2.2 Explicitly out of scope

This specification does **not** attempt full Lightpanda platform parity outside the MCP tool surface.

The following are not required by this design:

- CDP/WebSocket exposure;
- Playwright or Puppeteer compatibility;
- Lightpanda `serve`;
- Lightpanda `agent`;
- public proxy configuration;
- public custom-header configuration;
- public network interception configuration;
- public V8 runtime flag configuration;
- graphical screenshots;
- graphical viewport/device emulation;
- managed browser downloads;
- file-upload compatibility shims;
- legacy Playwright behavior emulation;
- Lightpanda MCP resources (`resources/list` / `resources/read`) as a public Local Workspaces feature.

The selected product requirement is **native MCP tool parity**, not every Lightpanda CLI/runtime capability.

---

## 3. Supersession

This specification supersedes the Browser Automation public-contract and runtime-transport portions of:

```text
docs/superpowers/specs/2026-08-27-lightpanda-native-mcp-wsl-design.md
```

Specifically, the following old design choices are no longer authoritative:

- one Lightpanda stdio process per logical tab;
- Local Workspaces-owned `browser_tabs` abstraction;
- public opaque refs such as `e1`, `e2`, ...;
- ref -> `nodeDetails` -> CSS selector compatibility routing;
- intentionally hidden `getCookies`, `getEnv`, `save`, and `session_*` tools;
- the public 39-tool browser compatibility contract;
- synthetic compatibility tools that do not exist in native Lightpanda MCP;
- statements that Lightpanda MCP must remain stdio-only.

The WSL ownership, pinned artifact, provisioning, integrity, telemetry-disable, Pause, audit, and non-root runtime principles from the prior design remain valid unless directly replaced here.

---

## 4. Source of truth

The implementation must use the **actual pinned binary contract** as the primary source of truth.

Observed on 2026-08-28 from the installed pinned runtime:

```text
lightpanda version
=> 0.3.7
```

The actual `tools/list` response contains **31 tools**. Direct descriptor inspection on the pinned binary shows every one of those 31 descriptors contains exactly `name`, `description`, and `inputSchema`; **none of the 31 advertises `outputSchema`**.

A direct HTTP-MCP `initialize` against that same `0.3.7` binary reports:

```text
serverInfo.name    = lightpanda
serverInfo.version = 0.1.0
```

Therefore `initialize.serverInfo.version` is **not** the Lightpanda binary-version authority for this integration. Binary pin verification must use the pinned runtime manifest, SHA-256 verification, and the actual `lightpanda version` command. MCP `initialize.serverInfo` remains protocol diagnostics only. Tool/schema contract verification is a separate gate.

This runtime evidence overrides stale assumptions in comments, previous specs, or compatibility adapters.

The current repository contains examples of assumptions that are no longer valid against the installed binary. For example, current compatibility code assumes several action tools require CSS-selector recovery, while the installed `0.3.7` contract directly accepts `backendNodeId` for those actions.

### 4.1 Contract snapshot

The implementation should introduce a generated, reviewed contract snapshot such as:

```text
src-tauri/resources/lightpanda-mcp-contract-0.3.7.json
```

The snapshot must contain:

- upstream tool name;
- upstream description;
- upstream input schema;
- required fields;
- enum values;
- defaults where upstream advertises them;
- output-schema presence/value. For pinned `0.3.7`, this is absent on all 31 descriptors and that absence is part of the reviewed contract.

Runtime verification must compare the discovered upstream contract to this pinned snapshot before declaring the browser runtime ready.

A missing tool, unexpected tool, or incompatible input schema is a runtime-version mismatch, not a partial-success state.

---

## 5. Product-level decisions

### 5.1 One browser tool family only

The public MCP surface must contain only:

```text
browser_lightpanda_*
```

There must be no duplicate compatibility aliases such as:

```text
browser_click
browser_fill
browser_snapshot
browser_tabs
browser_wait
browser_open
browser_close
browser_navigate
...
```

### 5.2 Native semantics over compatibility semantics

Local Workspaces must not emulate the previous Playwright-style contract when the native Lightpanda contract already exists.

Examples:

- use `goto`, not `browser_open` + `browser_navigate`;
- use `tree`, not `browser_snapshot`;
- use native `backendNodeId`, not `e1` refs;
- use `session_new/list/close`, not Local Workspaces logical tabs;
- use native `waitForSelector`, `waitForScript`, and `waitForState`, not a compatibility wait enum;
- use native `evaluate` semantics without automatically wrapping/invoking function literals;
- use native `hover`; remove synthetic hover;
- remove synthetic drag because Lightpanda MCP `0.3.7` does not advertise a drag tool.

### 5.3 One Lightpanda HTTP server

The browser runtime must use one managed Lightpanda MCP HTTP server process rather than one stdio process per page.

Native Lightpanda sessions provide isolated browsing contexts.

### 5.4 No hidden native tools

If the pinned Lightpanda `tools/list` advertises a tool, Local Workspaces must expose it under the `browser_lightpanda_*` namespace.

Local Workspaces may apply its normal user-controlled guards and safe transport validation, but it may not silently hide an advertised native tool.

---

## 6. Target architecture

```text
+------------------------------------------------------+
| ChatGPT / MCP client                                 |
+-----------------------------+------------------------+
                              |
                              | Local Workspaces MCP
                              v
+------------------------------------------------------+
| WorkspaceServer                                      |
|                                                      |
| - Pause gate                                         |
| - Browser Automation enabled gate                    |
| - input-schema validation                            |
| - navigation policy                                  |
| - redacted audit metadata                            |
+-----------------------------+------------------------+
                              |
                              | typed browser_lightpanda_* request
                              v
+------------------------------------------------------+
| LightpandaNativeService                              |
|                                                      |
| - lazy ensure_runtime()                              |
| - one HTTP server lifecycle                          |
| - upstream contract verification                     |
| - request IDs                                        |
| - per-session serialization                          |
| - optional Mcp-Session-Id routing                    |
| - no automatic replay                                |
+-----------------------------+------------------------+
                              |
                              | http://127.0.0.1:<ephemeral>/mcp
                              v
+------------------------------------------------------+
| Managed WSL2 distro                                  |
| LocalWorkspaces-Lightpanda                           |
|                                                      |
| localworkspaces user                                 |
| lightpanda mcp --host 127.0.0.1 --port <port>        |
|                                                      |
| default session                                      |
| named session A                                      |
| named session B                                      |
| ...                                                  |
+------------------------------------------------------+
```

---

## 7. Native upstream tool inventory

The pinned Lightpanda `0.3.7` runtime advertises these 31 tools:

```text
goto
search
markdown
html
links
evaluate
extract
tree
nodeDetails
interactiveElements
structuredData
detectForms
click
fill
scroll
waitForSelector
waitForScript
waitForState
hover
press
selectOption
setChecked
findElement
consoleLogs
getUrl
getCookies
getEnv
save
session_new
session_list
session_close
```

No implementation is considered complete unless all 31 are reachable through Local Workspaces.

---

## 8. Public Local Workspaces mapping

| Upstream Lightpanda tool | Public Local Workspaces tool |
|---|---|
| `goto` | `browser_lightpanda_goto` |
| `search` | `browser_lightpanda_search` |
| `markdown` | `browser_lightpanda_markdown` |
| `html` | `browser_lightpanda_html` |
| `links` | `browser_lightpanda_links` |
| `evaluate` | `browser_lightpanda_evaluate` |
| `extract` | `browser_lightpanda_extract` |
| `tree` | `browser_lightpanda_tree` |
| `nodeDetails` | `browser_lightpanda_node_details` |
| `interactiveElements` | `browser_lightpanda_interactive_elements` |
| `structuredData` | `browser_lightpanda_structured_data` |
| `detectForms` | `browser_lightpanda_detect_forms` |
| `click` | `browser_lightpanda_click` |
| `fill` | `browser_lightpanda_fill` |
| `scroll` | `browser_lightpanda_scroll` |
| `waitForSelector` | `browser_lightpanda_wait_for_selector` |
| `waitForScript` | `browser_lightpanda_wait_for_script` |
| `waitForState` | `browser_lightpanda_wait_for_state` |
| `hover` | `browser_lightpanda_hover` |
| `press` | `browser_lightpanda_press` |
| `selectOption` | `browser_lightpanda_select_option` |
| `setChecked` | `browser_lightpanda_set_checked` |
| `findElement` | `browser_lightpanda_find_element` |
| `consoleLogs` | `browser_lightpanda_console_logs` |
| `getUrl` | `browser_lightpanda_get_url` |
| `getCookies` | `browser_lightpanda_get_cookies` |
| `getEnv` | `browser_lightpanda_get_env` |
| `save` | `browser_lightpanda_save` |
| `session_new` | `browser_lightpanda_session_new` |
| `session_list` | `browser_lightpanda_session_list` |
| `session_close` | `browser_lightpanda_session_close` |

The public tool count for the Lightpanda family is therefore exactly **31**.

Assuming no unrelated MCP tool changes, the current server total changes from 53 tools to 45 tools:

```text
53 current total
-39 legacy browser tools
+31 browser_lightpanda tools
=45 total
```

---

## 9. Session routing model

### 9.1 Native Lightpanda behavior

Under HTTP MCP, Lightpanda has an always-present `default` browsing session.

`session_new` creates an isolated browser session.

`session_list` lists active sessions.

`session_close` closes a named session; `default` cannot be closed.

Every ordinary tool call is routed to a session through the HTTP header:

```text
Mcp-Session-Id: <session id>
```

Direct runtime verification found an important bootstrap behavior: an HTTP `initialize` request **without** this header creates a generated native session such as `s1`, which then appears in `session_list` alongside `default`. Local Workspaces must not create that hidden implementation session.

Therefore the internal MCP protocol bootstrap must explicitly use:

```text
Mcp-Session-Id: default
```

for `initialize`, `notifications/initialized`, startup `tools/list`, and all session-control calls. Ordinary public calls with omitted `session_id` must also send `Mcp-Session-Id: default` explicitly rather than relying on header omission.

### 9.2 Local Workspaces transport metadata

Because an outer MCP tool call cannot directly set the internal Lightpanda HTTP header, every public `browser_lightpanda_*` tool **except** the three session-control tools may accept one additional Local Workspaces metadata field:

```text
session_id?: string
```

This field is not forwarded inside the upstream JSON arguments.

Instead:

```text
public session_id
      |
      v
Mcp-Session-Id HTTP header
```

Omitted `session_id` means Local Workspaces explicitly sends the native `default` session header.

The following retain their native schemas without this routing field:

```text
browser_lightpanda_session_new
browser_lightpanda_session_list
browser_lightpanda_session_close
```

Those three session-control tools are always transported with `Mcp-Session-Id: default`. Creating a named native session does not trigger a second MCP `initialize`; later calls target it directly by sending that returned/native session ID in the header.

### 9.3 Session-id validation

A caller-supplied `session_id` is transport metadata and must be safe for use as an HTTP header value.

Local Workspaces must reject it before network I/O if it:

- is empty;
- exceeds 256 UTF-8 bytes;
- contains CR, LF, NUL, or other HTTP control characters.

Local Workspaces must not reinterpret valid names.

Unknown or closed session IDs are passed to Lightpanda and the native error is returned.

### 9.4 Concurrency

Calls against the same session should be serialized by Local Workspaces to avoid nondeterministic concurrent DOM mutations.

Different Lightpanda sessions may execute concurrently.

Use application-owned keyed locks:

```text
default -> mutex
session-a -> mutex
session-b -> mutex
```

`session_new/list/close` use a small global session-control lock.

The session registry in Local Workspaces is not authoritative. Lightpanda remains authoritative through `session_list`.

---

## 10. Native argument contracts

The public schema preserves upstream argument names and semantics. CamelCase upstream fields remain camelCase in JSON.

The only systematic Local Workspaces addition is optional `session_id` routing metadata described above.

### 10.1 Navigation and search

#### `browser_lightpanda_goto`

Required:

```text
url: string
```

Optional:

```text
timeout: integer
waitUntil: "load" | "domcontentloaded" | "networkalmostidle" | "networkidle" | "done"
session_id: string
```

#### `browser_lightpanda_search`

Required:

```text
query: string
```

Optional:

```text
timeout: integer
session_id: string
```

### 10.2 Page reading

#### `browser_lightpanda_markdown`

Optional:

```text
selector: string
backendNodeId: integer
maxBytes: integer
url: string
timeout: integer
session_id: string
```

#### `browser_lightpanda_html`

Optional:

```text
selector: string
backendNodeId: integer
url: string
timeout: integer
session_id: string
```

#### `browser_lightpanda_links`

Optional:

```text
url: string
timeout: integer
session_id: string
```

#### `browser_lightpanda_tree`

Optional:

```text
url: string
timeout: integer
backendNodeId: integer
maxDepth: integer
session_id: string
```

#### `browser_lightpanda_node_details`

Required:

```text
backendNodeId: integer
```

Optional Local Workspaces routing metadata:

```text
session_id: string
```

#### `browser_lightpanda_interactive_elements`

Optional:

```text
url: string
timeout: integer
session_id: string
```

#### `browser_lightpanda_structured_data`

Optional:

```text
url: string
timeout: integer
session_id: string
```

#### `browser_lightpanda_detect_forms`

Optional:

```text
url: string
timeout: integer
session_id: string
```

### 10.3 Scripting and extraction

#### `browser_lightpanda_evaluate`

Required:

```text
script: string
```

Optional:

```text
url: string
timeout: integer
save: string
session_id: string
```

Local Workspaces must pass the script as native Lightpanda input.

It must **not** transform:

```text
() => expression
```

into an immediately invoked function as the old compatibility layer does.

The public behavior is native Lightpanda `evaluate` behavior.

#### `browser_lightpanda_extract`

Required:

```text
schema: string
```

Optional:

```text
save: string
session_id: string
```

Unlike the old `browser_extract`, the public schema is the native Lightpanda schema string. Local Workspaces must not replace it with a JSON-object compatibility shape.

### 10.4 Element interaction

#### `browser_lightpanda_click`

Optional target pair:

```text
selector: string
backendNodeId: integer
```

Optional:

```text
session_id: string
```

#### `browser_lightpanda_fill`

Required:

```text
value: string
```

Target:

```text
selector?: string
backendNodeId?: integer
```

Optional:

```text
session_id: string
```

#### `browser_lightpanda_hover`

Target:

```text
selector?: string
backendNodeId?: integer
```

Optional:

```text
session_id: string
```

#### `browser_lightpanda_press`

Required:

```text
key: string
```

Optional:

```text
selector: string
backendNodeId: integer
session_id: string
```

If neither target field is supplied, native Lightpanda targets the document.

#### `browser_lightpanda_select_option`

Required:

```text
value: string
```

Target:

```text
selector?: string
backendNodeId?: integer
```

Optional:

```text
session_id: string
```

#### `browser_lightpanda_set_checked`

Required:

```text
checked: boolean
```

Native default advertised by upstream:

```text
checked = true
```

Target:

```text
selector?: string
backendNodeId?: integer
```

Optional:

```text
session_id: string
```

#### `browser_lightpanda_scroll`

Optional:

```text
backendNodeId: integer
x: integer
y: integer
session_id: string
```

No node means native window scrolling.

### 10.5 Waiting

#### `browser_lightpanda_wait_for_selector`

Required:

```text
selector: string
```

Optional:

```text
timeout: integer
session_id: string
```

#### `browser_lightpanda_wait_for_script`

Required:

```text
script: string
```

Optional:

```text
timeout: integer
session_id: string
```

The script is a native Lightpanda truthy expression. Local Workspaces must not rewrite it.

#### `browser_lightpanda_wait_for_state`

Required:

```text
state: "load" | "domcontentloaded" | "networkalmostidle" | "networkidle" | "done"
```

Optional:

```text
timeout: integer
session_id: string
```

### 10.6 Discovery and state

#### `browser_lightpanda_find_element`

Optional:

```text
role: string
name: string
session_id: string
```

Do not add the old Local Workspaces requirement that one criterion must be present unless the upstream pinned contract itself requires it.

#### `browser_lightpanda_console_logs`

Native arguments: none.

Optional Local Workspaces routing metadata:

```text
session_id: string
```

The result must be forwarded directly. Do not maintain the old compatibility pagination ring.

#### `browser_lightpanda_get_url`

Native arguments: none.

Optional:

```text
session_id: string
```

#### `browser_lightpanda_get_cookies`

Optional:

```text
url: string
all: boolean = false
session_id: string
```

Cookie values are intentionally returned because the user selected full native MCP tool parity.

They must never be copied into Local Workspaces audit metadata or desktop diagnostics.

#### `browser_lightpanda_get_env`

Optional:

```text
name: string
session_id: string
```

The upstream runtime itself restricts named lookup to `LP_*` variables.

Local Workspaces must not log returned values.

### 10.7 PandaScript persistence

#### `browser_lightpanda_save`

Required:

```text
path: string
script: string
```

Optional:

```text
session_id: string
```

The native relative-path rule remains authoritative.

Lightpanda should run with its working directory set to:

```text
/var/lib/local-workspaces/browser/pandascripts
```

This ensures a valid native relative save remains inside the application-owned distro.

Do not expose arbitrary host filesystem paths through this tool.

### 10.8 Native sessions

#### `browser_lightpanda_session_new`

Optional:

```text
name: string
```

Returns the native Lightpanda session result without Local Workspaces reinterpretation.

#### `browser_lightpanda_session_list`

Arguments: none.

Returns native Lightpanda session listing.

#### `browser_lightpanda_session_close`

Required:

```text
id: string
```

The native restriction that `default` cannot be closed remains authoritative.

---

## 11. Raw `backendNodeId` model

The old Local Workspaces opaque element ref system must be removed from the public browser path.

Delete the conceptual transformation:

```text
Lightpanda backendNodeId
        -> Local Workspaces e1/e2/e3
        -> resolve ref
        -> nodeDetails
        -> selector
        -> native action
```

The native path becomes:

```text
browser_lightpanda_tree
        -> backendNodeId: 47
        -> browser_lightpanda_click { backendNodeId: 47 }
```

The installed Lightpanda `0.3.7` contract advertises direct `backendNodeId` support for native actions such as click/fill/hover/press/selectOption/setChecked.

Local Workspaces must not force a CSS selector round trip when the caller supplies `backendNodeId`.

Native node lifetime/staleness behavior belongs to Lightpanda and is returned as native behavior.

### 11.1 PandaScript replayability caveat

The pinned native schemas accept `backendNodeId` directly, so exact parity requires exposing and forwarding it. However, Lightpanda's own `initialize` instructions for `0.3.7` warn that action calls recorded with `backendNodeId` are not reusable as replayable PandaScript calls and recommend CSS selectors for `click`, `fill`, `hover`, `selectOption`, and `setChecked` when replayability matters.

Local Workspaces must therefore:

- keep raw `backendNodeId` capability public and functional;
- never rewrite a caller-supplied native target solely for replayability;
- document selector targeting as the preferred workflow when the user intends to call native `save` and replay the resulting PandaScript;
- test both raw-ID parity and a selector-based replayable action flow.

---

## 12. HTTP MCP transport

### 12.1 Launch command

The managed runtime launches one Lightpanda process equivalent to:

```text
lightpanda mcp
  --host 127.0.0.1
  --port <app-selected-ephemeral-port>
  --log-level warn
  --cookie /var/lib/local-workspaces/browser/cookies/canonical.json
  --cookie-jar /var/lib/local-workspaces/browser/cookies/default.exit.json
  --http-cache-dir /var/lib/local-workspaces/browser/cache
  --http-cache-entry-limit 1000
```

Exact argument ordering is implementation detail; values that affect isolation are not.

The existing environment hardening remains mandatory:

```text
LIGHTPANDA_DISABLE_TELEMETRY=true
LIGHTPANDA_DISABLE_CORE_DUMP=1
```

Lightpanda runs as:

```text
localworkspaces
```

never as root.

### 12.2 Fixed working directory

Use WSL's fixed command working-directory facility rather than caller-controlled shell interpolation.

The Lightpanda process working directory must be:

```text
/var/lib/local-workspaces/browser/pandascripts
```

The runtime provisioner must create and own this directory for the `localworkspaces` user.

### 12.3 Bind policy

The Lightpanda MCP listener must bind only:

```text
127.0.0.1
```

Never:

```text
0.0.0.0
```

The Lightpanda port must never be included in Tailscale Funnel routing.

Only the Local Workspaces MCP server is remotely reachable through the product tunnel.

### 12.4 Port allocation

Local Workspaces selects an ephemeral Windows-loopback-safe port before launch.

If the chosen port conflicts inside WSL or the listener fails to become reachable, startup may choose a new port and retry a bounded number of times.

The port is runtime state only and is never model-controlled.

### 12.5 HTTP client

The internal client must use a dedicated app-owned `reqwest::Client` with:

- loopback-only base URL;
- bounded connect timeout;
- bounded request timeout;
- no redirects;
- `Content-Type: application/json`;
- `Accept: application/json, text/event-stream`;
- an explicitly selected `Mcp-Session-Id` header on protocol and tool calls;
- monotonically increasing JSON-RPC request IDs.

Do **not** use a generic MCP client transport that privately owns one negotiated session header for all calls. Local Workspaces must select `default` or a named native browsing session per request, so session-header routing remains application-owned and testable.

The client must understand both ordinary JSON responses and valid Streamable HTTP/SSE response framing if returned by the server.

### 12.6 Initialization

After process spawn, readiness requires:

1. the WSL-side listener is reachable from the Windows Local Workspaces process through `http://127.0.0.1:<port>/mcp`;
2. MCP `initialize` succeeds **with `Mcp-Session-Id: default`**;
3. the response session header is `default`;
4. `notifications/initialized` is sent with the same `default` header;
5. `tools/list` succeeds with the same `default` header;
6. `session_list` proves bootstrap did not create an implementation-only generated session such as `s1`;
7. discovered tool set equals the pinned 31-tool contract;
8. discovered input schemas match the embedded contract snapshot.

`initialize.serverInfo.version` must not participate in the binary version gate; direct runtime testing shows `0.3.7` currently reports MCP server version `0.1.0`. Binary pin validation remains the manifest/SHA-256/`lightpanda version` responsibility.

Windows-to-WSL loopback forwarding was verified on the target development host. Nevertheless, reachability is a mandatory startup check. If WSL launches the process but Windows loopback cannot reach it, startup fails closed; Local Workspaces must not broaden the bind address to `0.0.0.0` as a fallback.

Only after these checks may the service enter `Ready`/`Running` state.

---

## 13. Runtime lifecycle

### 13.1 Lazy startup

There is no public `browser_open` tool.

On the first allowed `browser_lightpanda_*` call:

```text
Pause check
 -> Browser Automation enabled check
 -> ensure managed WSL runtime
 -> ensure pinned Lightpanda binary
 -> start private HTTP MCP server if not running
 -> verify contract
 -> forward native tool call
```

### 13.2 No public global close

There is no public `browser_close` compatibility tool.

Normal browser-server shutdown occurs when:

- Browser Automation is disabled in desktop Settings;
- Pause is enabled;
- Reset browser data is requested;
- Remove browser runtime is requested;
- the desktop application exits.

Native named browsing sessions are closed with:

```text
browser_lightpanda_session_close
```

### 13.3 Graceful stop

For planned shutdown, prefer allowing Lightpanda to exit normally so `--cookie-jar` can flush.

A bounded graceful stop should be attempted before a forced managed-distro termination.

If graceful stop does not complete within the fixed deadline, Local Workspaces may terminate the app-owned distro after ownership verification.

### 13.4 Process loss

If the HTTP transport dies during a tool call:

- return a stable Local Workspaces process/transport error;
- mark the running Lightpanda service unhealthy;
- do not replay the failed call automatically;
- do not infer whether the upstream side effect occurred;
- allow a later independent tool call to lazily start a fresh server.

This rule applies even to calls believed to be read-only. Simplicity and duplicate-side-effect prevention take priority over transparent retries.

---

## 14. Cookies and browser state

### 14.1 Native session isolation

Named Lightpanda sessions are isolated browser contexts with their own page/cookies/memory according to upstream behavior.

Local Workspaces must not merge named-session cookies to make them behave like legacy tabs.

### 14.2 Default-session persistence

The server may continue using the existing app-owned cookie input/jar paths for the native default session.

Do not restore the old per-page custom cookie-merging architecture.

### 14.3 `getCookies` public access

`browser_lightpanda_get_cookies` intentionally exposes native cookie values to the authorized MCP caller.

This is a deliberate consequence of the selected full-native-tool-parity requirement.

Security rule:

> Cookie contents may appear in the tool result, but never in audit rows, browser runtime status, desktop diagnostics, logs, panic messages, or tracing metadata.

### 14.4 Web Storage

Public configuration of Lightpanda storage-engine flags is outside this specification.

If implementation chooses to enable the pinned runtime's supported SQLite storage internally, that choice must not alter any public native tool schema and must be documented separately.

---

## 15. `getEnv` security boundary

The native `getEnv` tool must be public.

However, Local Workspaces controls the environment of the Lightpanda process it starts.

The launch path should avoid injecting unrelated application secrets into the Lightpanda process environment.

In particular:

- do not map tunnel credentials into `LP_*` variables;
- do not map workspace secrets into `LP_*` variables;
- do not add secret-bearing `LP_*` variables merely to make `getEnv` return something;
- preserve upstream behavior for whatever `LP_*` environment is intentionally configured for Lightpanda.

The tool result remains native and unredacted to the authorized caller.

Audit and diagnostics never store its returned value.

---

## 16. PandaScript `save`

The native `save` tool is part of the required 31-tool surface.

### 16.1 Storage location

All relative save paths resolve from:

```text
/var/lib/local-workspaces/browser/pandascripts
```

### 16.2 Reset behavior

`Reset browser data` should clear volatile browser state such as cookies/cache/storage/artifacts but should **preserve explicitly saved PandaScripts** unless the UI explicitly states otherwise.

### 16.3 Remove-runtime behavior

`Remove browser runtime` unregisters the app-owned distro and therefore deletes PandaScripts stored inside that distro.

The confirmation copy must make that consequence explicit.

### 16.4 Audit

Never audit the script content.

Audit only bounded metadata such as:

- path byte length;
- path hash if needed for correlation;
- script byte length.

---

## 17. Local Workspaces policy

Full native tool parity does not remove Local Workspaces' user-controlled safety gates.

Every public `browser_lightpanda_*` call must pass, in order:

```text
1. Pause is OFF
2. Browser Automation is enabled
3. runtime is provisionable/ready
4. tool-specific Local Workspaces transport validation
5. upstream native call
```

### 17.1 URL policy

Any upstream argument named `url` that causes browser navigation must continue to pass the Local Workspaces navigation policy before forwarding.

At minimum, preserve the current policy that accepts absolute HTTP/HTTPS URLs and rejects local file/javascript schemes.

Examples:

```text
https://example.com       allowed
http://localhost:3000     allowed
file:///etc/passwd        denied
javascript:...            denied
```

This is a Local Workspaces security policy around the native tool, not a compatibility rewrite of the tool itself.

### 17.2 No model-controlled process arguments

Tool arguments may never become Lightpanda process CLI arguments.

Runtime launch configuration remains app-owned.

In particular, native tool callers cannot set:

- `--host`;
- `--port`;
- proxy flags;
- TLS flags;
- V8 flags;
- storage paths;
- cache paths;
- cookie input/jar paths;
- log configuration.

---

## 18. Upstream result forwarding

The native namespace must preserve Lightpanda tool behavior.

### 18.1 Success results

Forward the upstream `CallToolResult` content directly whenever possible.

Do not rebuild native results into compatibility-specific shapes such as:

```text
BrowserTextOutput
BrowserNavigationOutput
BrowserSnapshotOutput
BrowserConsoleOutput
BrowserNetworkOutput
```

The public `browser_lightpanda_*` declarations must also avoid inventing compatibility `outputSchema` contracts for variable native Lightpanda results. Live inspection of pinned `0.3.7` shows **0/31** native descriptors advertise `outputSchema`; therefore all 31 public native handlers must omit `outputSchema` as well. Contract verification must fail if a future pinned descriptor changes this assumption until the public contract is reviewed.

### 18.2 Upstream `isError`

If Lightpanda returns a valid MCP tool result with `isError=true`, preserve that result for the caller.

Do not replace normal native tool errors with compatibility wording merely because previous browser code used a stable Playwright-oriented error code.

### 18.3 Upstream JSON-RPC errors

Lightpanda may reject a valid `tools/call` at the JSON-RPC layer rather than returning `CallToolResult { isError: true }`. Direct runtime verification observed this for `getUrl` before any page is loaded, returning native code `-32604` with a `FrameNotLoaded` message.

For a syntactically valid upstream JSON-RPC error response:

- preserve the upstream numeric code and message for the authorized caller;
- convert it to a public `CallToolResult` with `isError=true` if the outer Local Workspaces MCP handler cannot directly forward the inner JSON-RPC error envelope;
- do not replace it with Playwright-era wording or `BROWSER_PROTOCOL_ERROR`;
- never write the raw upstream message into audit metadata, because native messages may contain page-derived or credential-adjacent data.

`BROWSER_PROTOCOL_ERROR` is reserved for malformed/mismatched JSON-RPC, invalid response framing, request-ID mismatch, or other failures where no trustworthy native Lightpanda error was received.

### 18.4 Local errors

Errors generated before or outside a valid upstream tool result remain Local Workspaces errors, including:

```text
ACCESS_PAUSED
BROWSER_AUTOMATION_DISABLED
BROWSER_RUNTIME_NOT_READY
BROWSER_RUNTIME_VERSION_MISMATCH
BROWSER_PROCESS_LOST
BROWSER_PROTOCOL_ERROR
INVALID_ARGUMENT
```

### 18.5 No automatic output truncation that changes native semantics

Do not arbitrarily truncate native tool output at the compatibility-layer limits used by the old browser API.

Normal MCP/server-wide transport safety limits may still apply, but they must be generic platform limits, not per-tool compatibility transformations.

For content size control, callers should use native Lightpanda arguments such as `maxBytes` where the upstream tool supports them.

---

## 19. Audit model

Every public `browser_lightpanda_*` invocation must produce one Local Workspaces MCP audit decision when the request reaches the handler.

The audit system must never store raw upstream output.

### 19.1 Forbidden audit content

Never store:

- cookie values;
- `getEnv` values;
- fill values;
- evaluate script contents;
- extraction schema contents;
- PandaScript contents;
- search-query contents;
- HTML/markdown/page text;
- console message contents;
- session names or IDs in raw form when user-supplied;
- URL query credential values.

### 19.2 Safe metadata examples

Permitted examples:

```text
tool = browser_lightpanda_goto
origin = https://example.com
wait_until = networkidle
timeout_present = true
```

```text
tool = browser_lightpanda_fill
target = backend_node_id
value_bytes = 24
session_present = true
```

```text
tool = browser_lightpanda_evaluate
script_bytes = 812
save_present = false
```

```text
tool = browser_lightpanda_get_cookies
all = true
url_origin = null
```

```text
tool = browser_lightpanda_save
path_bytes = 16
script_bytes = 4096
```

### 19.3 Session IDs

When audit correlation requires a session identifier, store only a bounded hash/fingerprint and byte length, never the raw user-supplied name.

---

## 20. MCP tool annotations

Tool annotations must not claim read-only behavior when native arguments can mutate state.

Examples:

- `goto`: not read-only;
- `search`: not read-only because it navigates;
- `markdown/html/tree/...` with optional `url`: conservatively not read-only;
- `evaluate`: not read-only because arbitrary page JavaScript can mutate state;
- `extract`: conservatively not read-only because native `save` bridge state may be used;
- `click/fill/hover/press/selectOption/setChecked/scroll`: not read-only;
- `waitFor*`: read-only hint may be true;
- `nodeDetails`: read-only;
- `findElement`: read-only;
- `getUrl`: read-only;
- `getCookies`: read-only;
- `getEnv`: read-only;
- `consoleLogs`: conservatively not read-only because reading drains the native buffer;
- `save`: not read-only;
- `session_new/session_close`: not read-only;
- `session_list`: read-only.

---

## 21. Browser runtime desktop status

The desktop Browser Automation card remains useful, but its copy and diagnostics must reflect the new architecture.

Recommended redacted status shape:

```text
enabled: bool
running: bool
backend: "lightpanda"
transport: "private_http"
runtime_state: ...
lightpanda_version: "0.3.7"
native_tool_count: 31
contract_verified: bool
wsl_installed: bool
distro_ready: bool
reboot_required: bool
repair_required: bool
capabilities:
  native_mcp: true
  native_http_sessions: true
  full_native_tool_parity: true|false
  visual_rendering: false
  screenshots: false
```

Do not expose:

- Lightpanda port;
- session IDs;
- current URLs;
- cookies;
- PandaScript contents;
- runtime filesystem paths.

### 21.1 Settings copy

Replace compatibility-oriented language such as:

```text
Semantic browser automation over a private native MCP channel
No file upload, managed downloads, dialogs, or device emulation
```

with wording centered on the actual native contract, for example:

```text
Lightpanda native MCP over a private local HTTP channel
31/31 native MCP tools available when the pinned runtime contract is verified
Native isolated browser sessions are supported
Lightpanda has no graphical rendering or browser screenshots
```

The UI should not imply that removed compatibility tools are still part of the product.

---

## 22. Runtime provisioning changes

The existing managed WSL2 provisioner remains the foundation.

Required changes:

1. continue pinning Lightpanda `0.3.7` by SHA-256;
2. replace stdio smoke verification with HTTP-MCP-capable verification;
3. validate the full 31-tool contract, not the old restricted `REQUIRED_NATIVE_TOOLS` set;
4. create `/var/lib/local-workspaces/browser/pandascripts`;
5. launch a single HTTP MCP server instead of one child per logical page;
6. expose runtime running state from the new native service rather than from `BrowserManager` page state.

### 22.1 Contract equality

The old concept:

```text
runtime must contain this restricted subset
```

becomes:

```text
runtime advertised tool set == pinned reviewed Lightpanda contract
```

Parity verification must fail both when a required tool is missing and when the pinned binary unexpectedly advertises an unreviewed tool.

---

## 23. Codebase migration

The current code was inspected before this specification was written.

The Browser Automation implementation is currently concentrated in:

```text
src-tauri/src/browser/lightpanda.rs
src-tauri/src/browser/driver.rs
src-tauri/src/browser/manager.rs
src-tauri/src/browser/mcp.rs
src-tauri/src/browser/runtime.rs
src-tauri/src/browser/wsl.rs
src-tauri/src/browser/policy.rs
src-tauri/src/browser/audit.rs
src-tauri/src/server.rs
src-tauri/src/commands.rs
src-tauri/src/agent_skills.rs
src-tauri/tests/browser_mcp_integration.rs
src-tauri/tests/browser_runtime_integration.rs
scripts/verify-browser-release.ps1
scripts/verify-lightpanda-runtime-manifest.ps1
src/lib/types.ts
src/pages/settings-page.tsx
```

### 23.1 Recommended resulting responsibilities

The implementation should converge toward focused modules similar to:

```text
src-tauri/src/browser/contract.rs
  pinned 31-tool contract metadata and schema verification

src-tauri/src/browser/http.rs
  JSON-RPC HTTP/SSE transport, request IDs, Mcp-Session-Id header

src-tauri/src/browser/lightpanda.rs
  one HTTP Lightpanda process lifecycle and native call forwarding

src-tauri/src/browser/runtime.rs
  WSL/rootfs/binary provisioning and integrity

src-tauri/src/browser/wsl.rs
  fixed-shape WSL ownership/launch/stop operations

src-tauri/src/browser/mcp.rs
  public browser_lightpanda argument structs and session routing metadata

src-tauri/src/browser/policy.rs
  URL and local transport validation only

src-tauri/src/browser/audit.rs
  redacted native-tool audit helpers
```

Exact file split may follow implementation constraints, but old compatibility state must not survive merely because it already exists.

### 23.2 Remove or retire compatibility architecture

The following concepts should disappear from the active native browser path:

```text
BrowserPageDriver
BrowserPageDriverFactory
PageLaunchSpec per logical page
ManagedPage
ActiveSession as Local Workspaces tab model
PageBinding
browser_tabs
new_page_id
spawn_page per tab
opaque eN refs
register_backend_node
resolve_ref
selector_for_ref
normalize_tree_text
normalize_backend_node_ids
synthetic hover
synthetic drag
compatibility browser_wait enum
compatibility history navigation helpers
compatibility console pagination ring
compatibility performance/network adapters
compatibility trace format for browser tools
compatibility screenshot/download/upload/dialog/devtools stubs
browser_run_steps compatibility dispatcher
```

Do not preserve dead code behind the public API after the native implementation is verified.

### 23.3 `server.rs`

Remove registration of the old 39 browser tools.

Register exactly 31 `browser_lightpanda_*` tools.

Do not create aliases for migration convenience; the user requirement explicitly rejects duplicate browser APIs.

### 23.4 `commands.rs`

Desktop runtime commands remain:

```text
browser_runtime_status
setup_browser_runtime
repair_browser_runtime
reset_browser_data
remove_browser_runtime
```

They are desktop/Tauri commands, not public browser MCP compatibility tools, so they do not violate the one-browser-tool-family requirement.

They must operate on the new HTTP native service lifecycle instead of page children.

`repair_browser_runtime`, `reset_browser_data`, and `remove_browser_runtime` must stop the running Lightpanda HTTP server before mutating or replacing runtime/browser files. Repair may then leave the service stopped; the next browser tool call lazily restarts it. This avoids replacing the pinned binary or browser-state files underneath an active native process.

### 23.5 Agent guidance and integration-test migration

`src-tauri/src/agent_skills.rs` currently teaches the removed compatibility workflow (`browser_get_status`, `browser_open`, `browser_snapshot`, opaque refs, compatibility console/network/screenshot tools). It must be rewritten in the same breaking change so first-party agent guidance only names tools that still exist.

The browser guidance should use native concepts:

```text
browser_lightpanda_tree
browser_lightpanda_get_url
browser_lightpanda_console_logs
browser_lightpanda_find_element
browser_lightpanda_node_details
browser_lightpanda_* native actions
browser_lightpanda_session_*
```

Visual verification remains separate through Local Workspaces Screen Capture (`list_capture_targets` / `capture_screen`) because Lightpanda is non-visual.

The existing browser integration suites are also compatibility-era tests and must be rewritten rather than incrementally preserving obsolete assertions:

```text
src-tauri/tests/browser_mcp_integration.rs
src-tauri/tests/browser_runtime_integration.rs
```

The release gate in `scripts/verify-browser-release.ps1` must stop counting old t01..t19 compatibility cases as proof of browser completeness and instead require the new 31-tool native parity/runtime suite.

---

## 24. Breaking API migration

This is intentionally a breaking MCP API change.

There is no deprecation window with both APIs registered.

After upgrade, clients must use native equivalents.

Examples:

| Old | New |
|---|---|
| `browser_open({url})` | `browser_lightpanda_goto({url})` |
| `browser_navigate({action:"url",url})` | `browser_lightpanda_goto({url})` |
| `browser_snapshot()` | `browser_lightpanda_tree()` |
| `browser_click({ref:"e2"})` | `browser_lightpanda_click({backendNodeId:2})` |
| `browser_fill({ref,text})` | `browser_lightpanda_fill({backendNodeId,value})` |
| `browser_tabs({action:"new"})` | `browser_lightpanda_session_new()` |
| `browser_tabs({action:"list"})` | `browser_lightpanda_session_list()` |
| `browser_tabs({action:"close"})` | `browser_lightpanda_session_close({id})` |
| `browser_evaluate({expression})` | `browser_lightpanda_evaluate({script})` |
| `browser_extract({schema:object})` | `browser_lightpanda_extract({schema:string})` |
| `browser_console()` | `browser_lightpanda_console_logs()` |

Legacy features with no native equivalent simply disappear rather than returning unsupported errors.

Examples:

```text
browser_screenshot
browser_downloads
browser_save_download_to_workspace
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

Screen Capture remains a separate Local Workspaces capability through its existing non-browser tools.

---

## 25. Testing strategy

### 25.1 Contract unit tests

Tests must assert:

```text
upstream snapshot tool count == 31
public mapping count == 31
all upstream names map exactly once
all public names are unique
no public name starts with legacy browser_ unless it starts browser_lightpanda_
```

More precisely, no removed compatibility browser tool may remain registered.

### 25.2 Schema parity tests

For every tool, compare the public forwarded native schema against the pinned upstream schema after removing the one allowed Local Workspaces metadata property:

```text
session_id
```

The comparison must validate:

- property names;
- property types;
- required sets;
- enum values;
- advertised defaults.

If an upstream future version adds a native property named `session_id`, CI must fail until routing metadata is redesigned.

### 25.3 HTTP transport tests

Use a fake HTTP MCP server to test:

- `initialize` is sent with `Mcp-Session-Id: default`;
- `notifications/initialized` and startup `tools/list` stay on `default`;
- ordinary calls with omitted public `session_id` explicitly send `default`;
- named calls send the exact validated native session ID;
- session-control calls always use `default`;
- ordinary JSON response;
- SSE-framed response;
- request ID correlation;
- invalid header characters rejected before request;
- HTTP timeout;
- connection loss;
- malformed JSON-RPC;
- upstream `CallToolResult.isError=true` preserved;
- valid upstream JSON-RPC errors preserve the native numeric code/message for the caller;
- malformed protocol errors become `BROWSER_PROTOCOL_ERROR`;
- no automatic replay.

### 25.4 Session tests

Against a real pinned Lightpanda runtime, verify:

1. bootstrap initialization with `Mcp-Session-Id: default` leaves `session_list` with `default` only — no hidden generated `s1` session;
2. `session_new` creates an isolated session;
3. calls with no public `session_id` affect only default;
4. calls with a named `session_id` route to that native session without a second initialize handshake;
5. two named sessions can hold different URLs at the same time;
6. cookies set in one session do not appear in another unless native upstream itself changes this behavior;
7. `session_close` removes the named session;
8. closing `default` returns the native upstream error;
9. `initialize.serverInfo.version` is not used as the binary `0.3.7` version assertion.

### 25.5 Native tool smoke matrix

Every one of the 31 public tools must have at least one integration path proving it reaches its corresponding upstream tool.

The integration suite may group tools by page fixture, but coverage must be explicit.

Suggested fixture capabilities:

- navigation target;
- links;
- semantic tree;
- form with text input/select/checkbox/radio;
- console logging;
- cookie setter;
- structured metadata;
- dynamic element for wait tests;
- evaluate/extract fixture;
- PandaScript save fixture.

### 25.6 `backendNodeId` tests

Real-runtime tests must prove that raw IDs returned by native `tree` can be passed directly to:

```text
click
fill
hover
press
selectOption
setChecked
markdown/html/tree subtree operations where supported
scroll
```

No selector-recovery call should be required by Local Workspaces when the raw ID is supplied.

A second real-runtime action test must use CSS selectors for `click`, `fill`, `hover`, `selectOption`, and `setChecked`, then exercise native `save` so the suite covers Lightpanda's documented replayable-PandaScript workflow as well as raw-ID parity.

### 25.7 Audit tests

For sensitive native tools, seed conspicuous secrets and assert they are absent from the audit database:

```text
getCookies -> cookie value not stored
getEnv -> LP_* value not stored
fill -> fill value not stored
evaluate -> script text not stored
extract -> schema text not stored
save -> script text not stored
search -> query text not stored
```

### 25.8 Lifecycle tests

Verify:

- browser disabled prevents startup;
- Pause prevents startup;
- Pause stops a running HTTP server;
- disabling Browser Automation stops a running server;
- app-owned runtime can restart lazily afterward;
- reset stops server before deleting browser state;
- remove-runtime stops server before unregistering distro;
- process crash never triggers automatic tool replay.

### 25.9 Tool discovery test

Start the actual Local Workspaces MCP server in test mode and assert the browser tool discovery set is exactly the 31 expected `browser_lightpanda_*` names.

A strong assertion is preferred over checking only individual names.

---

## 26. Acceptance criteria

Implementation is complete only when all of the following are true:

- [ ] Lightpanda remains pinned to `0.3.7` with SHA-256 `895339b02205171a181dde743ae0068bb4564884076feac8482baca9c212aa5a`.
- [ ] The managed WSL distro ownership and integrity checks remain fail-closed.
- [ ] Lightpanda runs as `localworkspaces`, never root.
- [ ] Telemetry and core dumps remain disabled.
- [ ] Runtime transport is private HTTP MCP on loopback, not stdio per page.
- [ ] One Lightpanda server process provides native sessions.
- [ ] Upstream `tools/list` is verified at startup.
- [ ] Protocol bootstrap uses `Mcp-Session-Id: default` and does not create a hidden generated native session.
- [ ] `initialize.serverInfo.version` is not used as the Lightpanda binary-version pin.
- [ ] Windows Local Workspaces can reach the WSL-bound `127.0.0.1` MCP listener before runtime status becomes Ready; failure never broadens the bind address.
- [ ] The verified upstream tool count is exactly 31 for the pinned contract.
- [ ] Local Workspaces exposes exactly 31 `browser_lightpanda_*` tools.
- [ ] Every upstream native tool maps to exactly one public tool.
- [ ] `getCookies` is public and functional.
- [ ] `getEnv` is public and functional.
- [ ] `save` is public and functional.
- [ ] `session_new`, `session_list`, and `session_close` are public and functional over HTTP.
- [ ] Ordinary native calls can target named sessions using optional `session_id` routing metadata.
- [ ] Omitted `session_id` routes to native `default`.
- [ ] Raw `backendNodeId` values are public and directly usable.
- [ ] Opaque `e1/e2/...` browser refs are absent from the new public API.
- [ ] No legacy compatibility `browser_*` tool remains registered.
- [ ] No alias layer temporarily duplicates old and new browser tools.
- [ ] Native upstream tool outputs are forwarded without compatibility reshaping.
- [ ] Public tool descriptors preserve upstream `outputSchema` presence/value and never invent compatibility output schemas when the pinned native tool omits one.
- [ ] Native upstream `isError` results are preserved.
- [ ] Valid upstream JSON-RPC tool errors preserve the native numeric code/message for the caller and are not mislabeled `BROWSER_PROTOCOL_ERROR`.
- [ ] Raw `backendNodeId` parity is tested, and selector-based action + native `save` replayability is tested separately.
- [ ] No failed transport call is automatically replayed.
- [ ] Pause stops active native browser automation.
- [ ] Disabling Browser Automation stops the Lightpanda server.
- [ ] Browser runtime status remains redacted.
- [ ] Cookie/env/fill/script/schema/PandaScript contents never enter audit rows.
- [ ] `save` is confined to the app-owned PandaScript working directory.
- [ ] Browser runtime setup/repair/reset/remove UI still works.
- [ ] Settings UI describes native HTTP sessions and full native tool parity accurately.
- [ ] Real-runtime integration tests cover all 31 tools.
- [ ] Contract parity tests fail if the pinned upstream contract drifts.

---

## 27. Release gate

A release containing this architecture must not ship until the following command families are green:

```text
cargo test --manifest-path src-tauri/Cargo.toml
npm test / frontend test suite as configured
npm run build
npm run verify:lightpanda-runtime
```

The real Lightpanda WSL integration suite must also run in required mode in CI/release verification, equivalent to the repository's existing `LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS=1` policy.

Release verification must explicitly print the parity result, for example:

```text
Lightpanda version: 0.3.7
Native tools discovered: 31
Native tools expected: 31
Public browser_lightpanda tools: 31
Legacy browser tools discovered: 0
Contract parity: PASS
```

A partial native surface is a release failure.

---

## 28. Design invariants

These invariants are intentionally short enough to use during code review:

1. **One public browser API:** `browser_lightpanda_*` only.
2. **One native server:** one Lightpanda HTTP MCP process, not one process per tab.
3. **Native sessions:** `Mcp-Session-Id`, not Local Workspaces tabs.
4. **Native IDs:** `backendNodeId`, not app-generated browser refs.
5. **31 means 31:** every pinned native tool is public.
6. **No compatibility invention:** if Lightpanda does not advertise a tool, do not keep a browser compatibility tool for it.
7. **No raw transport exposure:** the model calls typed Local Workspaces tools, never an arbitrary `call(tool,args)` passthrough.
8. **No process-argument injection:** model input never becomes Lightpanda CLI flags.
9. **Private transport:** Lightpanda binds loopback only and is not tunneled.
10. **Policy still applies:** Pause and Browser Automation enablement remain authoritative.
11. **Sensitive output is not audit data:** cookies/env/scripts may be returned natively but are never logged.
12. **No replay after uncertainty:** process/network failure never causes an automatic duplicate native call.

---

## 29. Final target

After this specification is implemented, Local Workspaces Browser Automation is no longer a Playwright-shaped compatibility abstraction running on top of Lightpanda.

It becomes a thin, policy-controlled native Lightpanda MCP integration:

```text
Local Workspaces responsibilities
  = ownership
  + provisioning
  + private transport
  + session routing metadata
  + policy
  + audit
  + lifecycle

Lightpanda responsibilities
  = browser semantics
  + DOM
  + navigation
  + actions
  + waits
  + cookies
  + evaluation
  + extraction
  + PandaScript save
  + browser sessions
```

That separation is the core architectural objective of this design.
