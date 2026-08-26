# Browser Automation and Debugging Design

**Date:** 2026-08-26
**Status:** Approved in chat

## Goal

Add a first-class browser automation and web-debugging subsystem to Local Workspaces so a connected engineering agent can reproduce, inspect, interact with, and verify web applications without requiring the user to manually drive a browser.

The subsystem must support navigation/search, deterministic element interaction, text entry, responsive viewport/device testing, page screenshots, console/network/DOM inspection, JavaScript evaluation, tracing/performance diagnostics, downloads/uploads under Local Workspaces confinement, and an optional visible browser/DevTools mode.

The default execution path is silent/background browser automation. Local Workspaces remains the only MCP app exposed to ChatGPT; browser automation is an internal capability behind the existing Local Workspaces policy, Pause, and audit boundaries.

## Product decisions approved in chat

1. Local Workspaces uses a dedicated browser profile. It never reuses the user's personal Chrome/Edge/Firefox profile.
2. The default browser is Playwright-managed bundled Chromium.
3. Chrome, Edge, and Firefox may be selectable alternatives when available.
4. Background/headless mode is the default operational mode. It must not open a normal browser window merely to automate or debug a page.
5. Visible Debug mode is optional. In that mode Chromium-family browsers may expose a real browser window and DevTools UI.
6. Background mode returns DevTools-equivalent data through MCP tools rather than opening a DevTools window.
7. Browser authorization is intentionally simple: one `Browser Automation` toggle. When enabled, the browser subsystem has the full browser automation/debug feature set defined here. There are no per-action permission toggles.
8. Browser Automation is enabled by default for the new settings schema. The user can disable it at any time in the desktop app.
9. Pause is a higher-priority kill switch and always blocks browser operations.
10. Browser page screenshots are a browser capability and do not depend on the separate OS-level `screen_capture_enabled` permission.
11. Browser downloads/uploads must not become a filesystem-policy bypass.
12. Release installers ship the required browser runtime instead of downloading Chromium on first use.
13. Release builds bump the application version before producing a new NSIS artifact.

## Why Playwright

Playwright provides the primitives required by this product without rebuilding a browser automation stack from raw Windows GUI automation or Chrome DevTools Protocol alone:

- Chromium, Chrome, Edge, Firefox, and WebKit-family automation support;
- headless and headed execution;
- persistent browser contexts backed by a dedicated user-data directory;
- structured accessibility snapshots and stable element references;
- navigation, forms, mouse/keyboard interaction, dialogs, tabs, frames, downloads, and uploads;
- viewport and device emulation;
- page/element screenshots;
- console and network observation;
- JavaScript evaluation;
- tracing with DOM snapshots, screenshots, console, and network information;
- Chromium-family Chrome DevTools Protocol access for deeper diagnostics.

The upstream Playwright MCP project already exposes a broad browser tool set and opt-in `devtools` capability. Local Workspaces should reuse that engine/runtime where practical while keeping its own stable MCP contract, policy enforcement, auditing, lifecycle, sandbox paths, and agent guidance.

## Architecture

```text
ChatGPT / MCP client
        |
        | Local Workspaces MCP only
        v
+-------------------------------+
| WorkspaceServer (Rust/rmcp)   |
| - browser_* tool schemas      |
| - Pause / policy checks       |
| - validation                  |
| - fail-closed audit           |
+---------------+---------------+
                |
                v
+-------------------------------+
| BrowserManager (Rust)         |
| - process lifecycle           |
| - session/page identity       |
| - profile/artifact paths      |
| - restart policy              |
| - workspace bridge checks     |
+---------------+---------------+
                |
      private parent-child MCP stdio
                |
                v
+-------------------------------+
| Browser Automation Sidecar    |
| - Playwright runtime          |
| - accessibility refs          |
| - page/context state          |
| - console/network capture     |
| - tracing/CDP                 |
+---------------+---------------+
                |
                v
+-------------------------------+
| Dedicated browser process     |
| Chromium by default           |
| headless/background default   |
| Local Workspaces profile only |
+-------------------------------+
```

The browser sidecar is an internal implementation detail. It does not expose any listening MCP endpoint and does not receive the Tailscale capability URL. `BrowserManager` launches it as a child process and speaks MCP over the child's piped stdin/stdout using rmcp's child-process transport. The child is placed in a Windows Job Object so the sidecar and browser process tree cannot outlive Local Workspaces.

## Component boundaries

### `BrowserManager` in Rust

`BrowserManager` is the authoritative runtime facade used by MCP handlers. It owns or coordinates:

- browser sidecar lifecycle;
- session identifiers;
- current browser engine and run mode;
- dedicated profile path;
- artifact sandbox path;
- browser startup/restart state;
- request/response timeouts;
- safe workspace-path bridging for uploads/download exports;
- validation of feature compatibility before a request reaches the sidecar;
- conversion of sidecar failures into stable Local Workspaces error codes.

It does not implement DOM selectors, Playwright element refs, or browser protocol details itself.

### Browser automation sidecar

Create a dedicated browser automation sidecar, separate from the existing Tailscale Funnel sidecar. The sidecar owns Playwright-specific behavior:

- persistent context creation;
- page/tab management;
- accessibility snapshots and ref resolution;
- element interaction;
- viewport/device emulation;
- page screenshots;
- console/network event buffering;
- DOM/style inspection;
- JavaScript evaluation;
- tracing and Chromium CDP operations;
- visible DevTools behavior when supported.

The first implementation uses a pinned official Playwright MCP/runtime as the internal browser engine behind a Local Workspaces adapter/proxy over private child-process stdio. Local Workspaces must not blindly forward the upstream tool schemas: the Rust MCP layer exposes the stable Local Workspaces tool contract defined below and calls only an allowlisted upstream subset. In particular, the upstream `browser_run_code_unsafe` tool is never exposed or invoked with model-supplied code because it executes in the Playwright server process rather than the page sandbox. User-facing `browser_evaluate` is limited to JavaScript evaluated in the web page context.

### Browser runtime packaging

The installed application must not depend on a globally installed Node.js, `npx`, or a first-run internet download.

The release bundle must include everything required to start the browser service and bundled Chromium. A practical first release may package:

- a pinned Node runtime used only by the internal browser sidecar;
- the compiled browser sidecar JavaScript and required production modules;
- the matching Playwright browser runtime;
- bundled Chromium artifacts required by the selected headless/headed mode.

The implementation plan may choose the exact packaging mechanism, but these invariants are mandatory:

1. no system Node dependency;
2. no first-run browser download;
3. Playwright and bundled Chromium versions are pinned together;
4. runtime files are installed under the application bundle/app-data rather than a user's personal browser directories;
5. browser startup works offline after installation, except of course navigation to remote websites.

## Settings schema

Bump the versioned `AppSettings` schema.

Add:

```text
browser_automation_enabled: bool = true
browser_engine: BrowserEngine = bundled_chromium
browser_run_mode: BrowserRunMode = background
```

Enums:

```text
BrowserEngine:
- bundled_chromium
- chrome
- edge
- firefox

BrowserRunMode:
- background
- visible_debug
```

`bundled_chromium` is the default and required release path.

The desktop Settings page adds a **Browser Automation** section:

- Browser Automation toggle;
- Browser selector;
- Run mode selector;
- short copy explaining that Local Workspaces uses its own profile and does not access the user's normal browser profile;
- status/diagnostic text when a selected optional browser is unavailable.

Changing engine or run mode affects the next browser launch. If a browser is already running, the UI may explicitly restart the browser session or mark the change as pending; it must not silently mutate a live context in an undefined state.

### MCP settings mutation

The existing `update_agent_settings` tool does not gain permission to enable/disable Browser Automation or select an installed browser. Browser Automation is a desktop-owned capability because its single toggle controls a broad external-interaction surface.

A future safe settings tool may be designed separately. The browser feature itself does not require one.

## Browser profile and session model

### Dedicated persistent profile

Use a Local Workspaces-owned persistent profile such as:

```text
<app-data>/browser/profiles/<browser-engine>/
```

This profile stores browser automation cookies, localStorage, IndexedDB, and other browser session state. It is separate from all personal browser profiles.

Requirements:

- never point Playwright at Chrome/Edge/Firefox default user-data directories;
- never copy personal browser cookies or credentials into the automation profile automatically;
- each browser engine/channel gets its own dedicated profile directory so Chromium-family and Firefox profile formats are never mixed;
- only one persistent automation profile instance may be open at a time;
- normal application restart preserves the selected engine's profile so login can persist;
- deleting/resetting the automation profile is a desktop UI operation, not an MCP action in the first release.

### Browser session

`browser_open` ensures a browser process/context exists and returns a Local Workspaces `session_id`. A session owns tabs/pages and ephemeral debug buffers.

A session ends when:

- `browser_close` is called;
- the user disables Browser Automation;
- the user activates Pause and the manager chooses to terminate the active browser for stronger containment;
- the app exits;
- the browser process becomes unrecoverable.

Pause must immediately block new browser calls. The implementation should terminate or freeze the sidecar/browser promptly enough that ongoing autonomous action cannot continue after Pause.

### Page/tab identity

Expose Local Workspaces page ids rather than raw Playwright object references. Page ids are session-scoped.

Element refs returned by `browser_snapshot` are page-state-scoped. Navigation or a material DOM rerender may invalidate refs. Ref invalidation returns `BROWSER_STALE_REF`, after which agent guidance requires a new snapshot before retry.

## MCP browser tool surface

The initial release exposes the following tools. Tool descriptions should teach safe composition and recovery so clients can work agentically without loading a separate manual.

### Lifecycle and status

#### `browser_get_status`

Read-only. Returns:

- browser automation enabled state;
- Pause state;
- configured browser engine;
- actual browser engine/channel;
- background vs visible mode;
- browser running state;
- current `session_id` if running;
- active page id, URL, title, and viewport when available;
- sidecar/browser version metadata safe for diagnostics.

#### `browser_open`

Ensures the configured browser is running. Optional initial URL. Returns session/page status.

Rules:

- `Browser Automation = OFF` -> `BROWSER_AUTOMATION_DISABLED`;
- Pause -> `ACCESS_PAUSED`;
- default mode is headless/background;
- launch uses the dedicated Local Workspaces profile;
- no personal profile attach/reuse mode exists.

#### `browser_close`

Closes the active browser context/process cleanly and ends the session.

### Tabs and navigation

#### `browser_tabs`

Supports list/open/switch/close operations on session tabs/pages.

#### `browser_navigate`

Supports:

- URL navigation;
- back;
- forward;
- reload.

Remote HTTP/HTTPS and localhost development URLs are allowed. `file://` navigation is denied in the first release so browser navigation cannot directly become an unrestricted filesystem read path.

#### `browser_search`

Convenience operation that navigates the active/new page to a web search query. The first release may default to Google Search, with the search provider an optional future setting.

### Page understanding and interaction

#### `browser_snapshot`

Returns an LLM-oriented accessibility/DOM snapshot with stable element refs where possible. This is the primary way an agent locates controls before interaction.

The snapshot should be bounded by output limits and support focused/subtree retrieval when the upstream engine provides it.

#### `browser_click`

Clicks an element ref/target.

#### `browser_fill`

Fills text into an input/textarea/contenteditable target. Sensitive text is never copied into audit detail.

#### `browser_press`

Sends a keyboard key/chord to a target/page.

#### `browser_select`

Selects one or more options.

#### `browser_hover`

Hovers a target.

#### `browser_drag`

Drags source to target.

#### `browser_wait`

Wait primitive supporting the smallest useful set of conditions:

- element/ref visible/hidden;
- text present/absent;
- URL match;
- load state;
- bounded duration.

Avoid a generic unbounded sleep primitive.

#### `browser_dialog`

Accepts or dismisses alert/confirm/prompt dialogs, optionally providing prompt text.

### Responsive and visual verification

#### `browser_set_viewport`

Sets explicit viewport width/height. Used for arbitrary responsive checks such as:

- 320×568;
- 375×812;
- 768×1024;
- 1440×900;
- any valid bounded custom size.

Returns the effective viewport after application.

#### `browser_emulate_device`

Applies a Playwright device preset or explicit emulation properties where supported:

- viewport;
- device scale factor;
- touch/mobile state;
- orientation;
- user agent.

Device emulation must make its effective state queryable from `browser_get_status` or the tool result.

#### `browser_screenshot`

Captures browser-rendered content and returns an MCP image block plus metadata.

Modes:

- viewport;
- full page;
- element/ref.

Optional `max_edge` or equivalent output bound prevents pathological image sizes.

This is distinct from `capture_screen`: it captures the browser page directly and therefore works in headless/background mode without OS Screen Capture permission.

### DevTools-equivalent debugging tools

#### `browser_console`

Returns buffered console messages with bounded output and filters such as level/page/since-cursor.

Safe fields include:

- level;
- message text;
- source URL/location when available;
- timestamp/order cursor.

Console content is returned to the caller but not duplicated wholesale into Local Workspaces audit rows.

#### `browser_network`

Returns bounded request/response metadata with filters for URL, resource type, status, failure, and time cursor.

Safe response fields include:

- method;
- URL;
- resource type;
- status;
- timing summary when available;
- failure reason;
- selected non-sensitive headers when explicitly needed.

Do not return cookies/authorization headers by default. Response bodies are excluded from the first release unless a later explicitly scoped debugging API is designed.

#### `browser_dom`

Inspects one target/ref and returns requested structural/debug information such as:

- tag/role/name;
- attributes with sensitive-value redaction;
- bounding box;
- selected computed styles;
- accessibility properties;
- limited outerHTML when requested and bounded.

#### `browser_evaluate`

Evaluates JavaScript in the page context for debugging/inspection.

Because Browser Automation is a full browser capability when enabled, arbitrary page-context JavaScript is allowed. The result must be JSON-serializable/bounded. Browser-side JS must not be granted special filesystem/process primitives beyond normal browser sandbox capabilities.

#### `browser_performance`

Returns available performance diagnostics such as:

- Navigation Timing;
- Resource Timing summaries;
- paint timing;
- long-task information when observable;
- Chromium-specific performance/CDP metrics where supported.

Unsupported metrics return partial capability metadata rather than failing the entire request where practical.

#### `browser_trace`

Supports start/stop/status for a browser trace.

Trace output may contain DOM snapshots, screenshots, console, network metadata, and action details. Trace artifacts are written only into the browser artifact sandbox and returned to the agent as metadata/handle, not arbitrary filesystem paths.

#### `browser_devtools`

Visible Debug mode only. On Chromium/Chrome/Edge, opens or focuses the real developer tools UI when supported by the selected launch mode. This is a Local Workspaces launch/configuration feature rather than an assumption that upstream Playwright MCP exposes a real-DevTools-window tool; current Playwright launches use Chromium arguments such as `--auto-open-devtools-for-tabs` when compatible.

In Background mode it returns `BROWSER_FEATURE_UNSUPPORTED_IN_MODE` with guidance to use `browser_console`, `browser_network`, `browser_dom`, `browser_evaluate`, `browser_performance`, and `browser_trace` instead.

Firefox may not support this tool in the first release.

### File transfer tools

#### `browser_upload`

Sets files for a browser file chooser/input.

Accepted sources:

1. browser artifact/profile sandbox files explicitly intended for upload; or
2. files under an approved Local Workspaces workspace with Read permission.

The Rust layer resolves and validates the path before the sidecar receives it. Absolute arbitrary filesystem paths from the model are rejected.

#### `browser_downloads`

Lists browser-managed downloads in the current session with ids and safe metadata.

Downloads land in:

```text
<app-data>/browser/artifacts/<session-id>/downloads/
```

The tool does not silently move files into a project.

#### `browser_save_download_to_workspace`

Copies a completed browser download into an approved workspace path only after applying the same workspace Write/path-confinement rules as normal file tools.

This operation records workspace id/path and safe file metadata in audit, not file contents.

### Batched flow tool

#### `browser_run_steps`

Runs a small bounded sequence of browser actions to reduce MCP round trips for deterministic flows.

Allowed steps are a subset of ordinary browser operations such as:

- navigate;
- snapshot/focused lookup;
- click;
- fill;
- press/select;
- wait;
- set viewport;
- screenshot.

Requirements:

- hard maximum step count;
- per-step result and failure index;
- stops on first failure by default;
- does not automatically replay the batch after a browser crash;
- does not add operations unavailable as individual tools;
- audit records the step types/count but not filled text values.

The agent should prefer individual tools while diagnosing ambiguous failures and use `browser_run_steps` for already-understood repetitive flows.

## Browser compatibility matrix

### Bundled Chromium

Required and fully supported:

- background/headless;
- visible debug;
- core navigation/interaction;
- snapshots;
- responsive/device emulation;
- screenshots;
- console/network/DOM/evaluate;
- trace;
- Chromium/CDP performance diagnostics;
- real DevTools UI in Visible Debug mode where launch semantics allow it.

### Google Chrome / Microsoft Edge

Optional installed-browser channels. Expected to support most Chromium features. Startup may fail or lose capabilities under enterprise browser policies; Local Workspaces reports a stable diagnostic rather than silently falling back to personal-profile attachment.

### Firefox

Optional installed/Playwright-supported Firefox path for core automation, screenshots, console/network, and standard page evaluation.

Chromium/CDP-specific metrics and real Chromium DevTools behavior return `BROWSER_FEATURE_UNSUPPORTED` rather than emulating misleading results.

## Background/headless behavior

The product requirement is that normal browser debugging should not annoy the user with windows appearing on the desktop.

In Background mode:

- launch browser headless;
- do not open browser DevTools UI;
- use Playwright APIs/trace/CDP to retrieve equivalent debug data;
- do not create a visible top-level browser window;
- screenshots are generated from the page renderer and returned through MCP.

An automated Windows test should verify that a background launch does not create a visible top-level browser window owned by the launched browser process tree.

Visible Debug mode is intentionally different: opening a real browser window/DevTools is expected and user-selected.

## Artifact sandbox

Use a session-scoped browser artifact directory:

```text
<app-data>/browser/artifacts/<session-id>/
├─ downloads/
├─ traces/
├─ screenshots/   # only when persistence is explicitly required internally
└─ diagnostics/
```

Most MCP screenshots should remain memory-returned image blocks and need not be persisted.

### Retention

Artifacts must not grow without bound. The implementation must provide deterministic cleanup, for example:

- remove incomplete/transient artifacts on browser close when not referenced;
- retain only a bounded number/age/total size of completed trace/download sessions;
- perform cleanup at startup and after session close;
- never delete files already exported into a user workspace.

Exact initial retention constants may be chosen in implementation but must be centrally defined and covered by tests.

## Network and URL policy

Browser Automation is intentionally a broad external interaction capability when enabled. It may access normal internet and localhost development endpoints.

Still enforce these invariants:

- Pause blocks all browser requests/actions;
- `file://` navigation is denied in the initial release;
- the upstream MCP child is private stdio-only and Local Workspaces never exposes or calls its process-level unsafe-code tool with model-supplied code;
- browser page JavaScript runs only through page-context evaluation and does not gain Node/process/filesystem primitives;
- downloads remain in browser sandbox until explicitly exported;
- upload paths are validated by Rust before the browser receives them;
- no browser operation receives Local Workspaces/Tailscale capability secrets unless the user/web application itself supplies them through normal page interaction.

## Auditing and redaction

Every browser MCP request that reaches a handler produces one Local Workspaces audit row. Browser audit remains fail-closed: if the audit row cannot be written, the MCP operation must not report successful completion.

Audit may include:

- browser operation name;
- session/page id;
- browser engine/run mode;
- URL origin or redacted URL metadata where appropriate;
- action type;
- target/ref id;
- duration/status;
- viewport dimensions;
- screenshot dimensions/encoded byte count/hash;
- network/console result counts;
- trace/download artifact id and size;
- batch step types/count.

Audit must not include:

- passwords or filled form text;
- cookies;
- Authorization headers/tokens;
- browser storage values;
- response bodies;
- screenshot bytes;
- trace contents;
- downloaded file contents;
- full console/network payload dumps when they may contain page secrets.

## Stable error model

Add stable browser errors at minimum:

- `BROWSER_AUTOMATION_DISABLED`
- `BROWSER_NOT_RUNNING`
- `BROWSER_START_FAILED`
- `BROWSER_PROCESS_LOST`
- `BROWSER_TARGET_NOT_FOUND`
- `BROWSER_STALE_REF`
- `BROWSER_NAVIGATION_TIMEOUT`
- `BROWSER_ACTION_TIMEOUT`
- `BROWSER_FEATURE_UNSUPPORTED`
- `BROWSER_FEATURE_UNSUPPORTED_IN_MODE`
- `BROWSER_INVALID_VIEWPORT`
- `BROWSER_UPLOAD_DENIED`
- `BROWSER_DOWNLOAD_NOT_FOUND`
- `BROWSER_DOWNLOAD_DENIED`
- `BROWSER_ARTIFACT_FAILED`
- `BROWSER_PROTOCOL_ERROR`

`ACCESS_PAUSED` remains the higher-priority Pause error.

Sidecar implementation errors must be mapped to stable Local Workspaces codes. Raw stack traces or internal filesystem paths should not be returned to remote MCP clients by default.

## Recovery behavior

### Stale element refs

If a ref is stale after navigation/rerender:

1. return `BROWSER_STALE_REF`;
2. agent guidance requires `browser_snapshot` again;
3. retry only against a newly resolved target.

Do not guess coordinates as an automatic fallback.

### Browser process loss

`BrowserManager` may automatically restart the browser/sidecar once for recovery when the failure happens before a side-effecting browser action is known to complete.

It must not automatically replay actions such as:

- click on submit/purchase/delete controls;
- form submission;
- upload;
- download-triggering actions;
- arbitrary `browser_evaluate` code with side effects.

After ambiguous process loss, report `BROWSER_PROCESS_LOST` and let the agent inspect current state before acting again.

### Timeouts

Navigation and action timeouts are bounded and centrally configured. Tool arguments may request a shorter timeout; remote MCP callers cannot request an unbounded operation.

## Agent operating guidance

Extend `AGENT_INSTRUCTIONS` and relevant prompts so browser debugging follows an evidence-driven loop:

1. call `browser_get_status` when browser state matters;
2. open/navigate the dedicated browser only when relevant to the task;
3. use `browser_snapshot` before deterministic interactions;
4. prefer refs/semantic targets over coordinates;
5. after an interaction, verify by snapshot, URL/state, console/network evidence, or screenshot depending on the bug;
6. use responsive viewport checks for layout/responsiveness tasks;
7. use console/network/DOM/performance/trace for debugging rather than opening visible DevTools in background mode;
8. when a stale ref occurs, snapshot again;
9. never use shell/PowerShell to bypass `BROWSER_AUTOMATION_DISABLED` or Pause;
10. never claim a UI/browser bug fixed solely from a successful build when browser evidence is practical and Browser Automation is enabled.

A browser-focused prompt such as `debug-web` may be added if it meaningfully improves orchestration, but the primary contract remains server instructions + tool descriptions.

## UI and diagnostics

The Settings Browser Automation card should expose enough diagnostic state to help users understand failures without revealing sensitive browsing data:

- configured browser;
- background/visible mode;
- installed/available state for optional Chrome/Edge/Firefox;
- bundled Chromium version;
- browser sidecar/runtime version;
- profile location via an explicit copy/reveal control if useful;
- reset automation profile action with confirmation;
- current browser process state.

The global diagnostics report may include browser enabled/configured/running/version fields but must not include browsing history, cookies, active page content, console messages, or downloaded filenames unless explicitly requested by a separate user action.

## Testing strategy

Follow RED → GREEN → REFACTOR.

### Unit tests

Cover:

- settings migration/defaults;
- browser engine/run mode serialization;
- BrowserManager state transitions;
- Pause precedence;
- disabled toggle handling;
- stable sidecar-error mapping;
- URL validation (`file://` denied, localhost/HTTP(S) allowed);
- upload path confinement;
- download export confinement;
- artifact retention decisions;
- no sensitive audit fields.

### Fake sidecar MCP integration tests

Use an injectable browser backend/fake sidecar to verify tool contracts without requiring Chromium for every Rust test:

- tool discovery exposes browser tools;
- disabled/Pause errors;
- browser status/open/close;
- stale ref recovery contract;
- screenshot returns MCP image content;
- console/network output shapes;
- batch step failure indexing;
- audit rows are fail-closed and redacted;
- file bridge respects workspace permissions.

### Real bundled Chromium integration tests

Run a local deterministic test web application and drive the actual shipped browser stack through the same BrowserManager protocol:

1. start background browser;
2. navigate to local test page;
3. snapshot and resolve controls;
4. fill/click;
5. verify DOM/page state;
6. observe a known console message/error;
7. observe a known network request/response;
8. execute bounded JavaScript inspection;
9. resize and verify each required viewport;
10. capture screenshot and validate image metadata;
11. run trace and verify artifact creation;
12. exercise download sandbox + workspace export;
13. close cleanly.

Responsive matrix minimum:

- 320×568
- 375×812
- 768×1024
- 1440×900

### Background-window test

On Windows, launch Background mode and assert no visible top-level browser window is present for the browser process tree.

### Crash/recovery test

Terminate the browser process during a controlled non-side-effecting scenario and verify:

- process loss is detected;
- one safe restart path works;
- no ambiguous action is automatically replayed;
- subsequent status accurately reflects the recovered/new session.

### Browser selection tests

Bundled Chromium is mandatory CI/release coverage.

Chrome/Edge/Firefox tests may be conditional on availability, but unsupported capability behavior must be deterministic and tested through fakes even when those browsers are absent.

### Full repository verification

Before release:

- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace`;
- frontend TypeScript/Vite build;
- existing Tailscale Go tests/vet + sidecar build;
- browser sidecar unit/integration tests;
- bundled Chromium smoke test;
- NSIS release build.

A new release build increments the application version first and produces a version-distinct installer filename.

## Security invariants

1. Pause always wins.
2. Browser Automation OFF blocks all `browser_*` actions regardless of unrestricted filesystem/execution mode.
3. Browser Automation ON grants the full browser feature set described here; there are no hidden per-action confirmations.
4. Browser Automation does not grant unrestricted filesystem access.
5. OS Screen Capture permission is independent from browser-rendered screenshots.
6. No personal browser profile reuse.
7. No arbitrary local file navigation in the first release.
8. Upload/download filesystem bridges are validated in Rust against browser sandbox/workspace policy.
9. The sidecar is private/local and cannot outlive the desktop application process tree.
10. Browser output may contain sensitive page content, but audit logging remains metadata-focused and redacted.
11. Browser process crashes never justify replaying an ambiguous side-effecting action automatically.
12. Browser automation must not be bypassed through `run_command`/`run_powershell` when disabled or paused.

## Non-goals for the first release

- Reusing or attaching to the user's personal browser profile.
- Browser extension mode for personal Chrome/Edge tabs.
- Generic desktop GUI automation outside browser pages.
- CAPTCHA bypass.
- Stealth/anti-bot evasion features.
- Unbounded scraping/crawling framework.
- Returning arbitrary network response bodies by default.
- Arbitrary `file://` filesystem browsing.
- Recording microphone/camera input.
- Replacing Local Workspaces MCP with a separately configured Playwright MCP app.

## Success criteria

The feature is complete when a fresh NSIS installation can, without a system Node dependency or first-run browser download:

1. run bundled Chromium invisibly in Background mode using the dedicated Local Workspaces profile;
2. let the MCP agent navigate/search/click/fill/select/press/hover/drag/wait and manage tabs;
3. inspect a page through semantic snapshots;
4. test arbitrary viewport sizes and device presets;
5. return page screenshots through MCP image content;
6. expose console/network/DOM/evaluate/performance/trace debugging data without opening DevTools UI;
7. optionally run a visible Chromium-family browser and DevTools when the user selects Visible Debug mode;
8. safely handle uploads/downloads without escaping workspace/browser sandbox policy;
9. obey Browser Automation toggle and Pause deterministically;
10. leave a redacted fail-closed audit trail;
11. survive/recover from non-ambiguous browser process loss according to the rules above;
12. pass the repository and real-browser verification suite;
13. build a bumped-version NSIS installer containing the required browser runtime.
