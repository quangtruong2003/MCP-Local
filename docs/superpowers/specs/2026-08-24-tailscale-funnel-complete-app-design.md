# Local Workspace MCP Manager — Tailscale Funnel Complete App Design

Date: 2026-08-24
Status: Approved direction — Tailscale Funnel selected
Supersedes for connection/UX scope: `docs/superpowers/specs/2026-08-23-local-workspace-mcp-manager-design.md`

## 1. Goal

Turn the current MVP control panel into a polished Windows desktop application where a normal user can:

1. Add one or more local workspace folders.
2. Press one primary **Start** button.
3. Complete Tailscale sign-in only on first use.
4. Receive a stable public HTTPS MCP endpoint under `*.ts.net`.
5. Copy that endpoint into ChatGPT.
6. Stop and restart the app later while keeping the same endpoint identity.
7. Grant an explicit per-workspace **Execute** capability so ChatGPT can run approved developer commands and, only when separately enabled, Full PowerShell with that workspace as the working directory.
8. Understand connection, workspace permissions, execution risk, security state, and recent activity without needing to understand localhost ports, child processes, or MCP internals.

The stable endpoint is provided by **Tailscale Funnel** through an app-specific embedded Tailscale node implemented as a small Go `tsnet` sidecar.

## 2. Product principles

- **One primary action:** Start/Stop is the main connection control.
- **First run may require setup; normal runs must not.** Tailscale login and Funnel enablement are onboarding events, not routine workflow.
- **Stable identity:** preserve the app-specific `tsnet` node state and hostname so the `*.ts.net` FQDN remains stable across app restarts.
- **Security remains fail-closed:** the existing Rust core policy pipeline remains the only filesystem doorway.
- **Execution is a separate privilege:** `Execute` is OFF by default and is never implied by Read or Write.
- **No fake sandbox claims:** setting `cwd` to a workspace does not confine PowerShell or a child process to that folder. The product must describe Approved Commands and Full PowerShell as code-execution capabilities, not filesystem sandboxes.
- **Full PowerShell is advanced:** arbitrary PowerShell requires Execute ON plus an explicit `Full PowerShell` workspace mode and a risk acknowledgement in the UI.
- **Do not expose raw no-auth `/mcp` publicly.** The Funnel sidecar exposes only a capability-protected public route and rewrites it to the loopback MCP route.
- **Local MCP stays loopback-only:** Rust MCP continues binding to `127.0.0.1:<ephemeral-port>`.
- **The UI reports verified states, not process-spawn guesses.** `Online` means the sidecar is authenticated, Funnel listener is active, and end-to-end public health verification succeeded.
- **No dependency on Tailscale Desktop/CLI at runtime.** The app bundles its own `tsnet` sidecar.
- **Windows-first:** Windows 10/11 is the supported target for this milestone.

## 3. High-level architecture

```text
ChatGPT Web
    |
    | HTTPS
    v
https://<hostname>.<tailnet>.ts.net/<capability>/mcp
    |
    v
Tailscale Funnel
    |
    v
Go tsnet sidecar (Funnel-only listener :443)
    |  validate capability path
    |  reverse proxy /<capability>/mcp -> /mcp
    v
http://127.0.0.1:<ephemeral>/mcp
    |
    v
Rust rmcp + axum MCP server
    |
    v
Existing fail-closed CoreServices policy pipeline
    |
    v
Approved local workspaces only
```

### 3.1 Process layout

The installed application contains:

- Tauri/Rust main process.
- React UI embedded by Tauri.
- New `tailscale-funnel-sidecar.exe`, built from Go and bundled as a Tauri external binary.

The sidecar owns only Tailscale/Funnel/network-publication responsibilities. It never accesses workspace files.

### 3.2 Why tsnet sidecar instead of Tailscale CLI

The sidecar uses `tailscale.com/tsnet` because it:

- runs its own userspace Tailscale node;
- does not require the Tailscale desktop client or system daemon;
- does not require Administrator elevation simply to run tsnet;
- supports persistent state with `tsnet.Server.Dir`;
- supports a fixed `Server.Hostname`;
- supports public Funnel with `Server.ListenFunnel`;
- exposes user-facing login messages through `Server.UserLogf`;
- exposes the FQDN through `Server.CertDomains()` after the node is online.

Official reference:
- https://tailscale.com/docs/reference/tsnet-server-api
- https://tailscale.com/docs/features/tsnet
- https://tailscale.com/docs/features/tailscale-funnel

## 4. Stable URL model

Use a fixed logical hostname, default:

```text
local-workspace-mcp
```

The resulting Tailscale certificate/DNS domain is expected to resemble:

```text
local-workspace-mcp.<tailnet>.ts.net
```

The public MCP connection URL shown to the user is:

```text
https://local-workspace-mcp.<tailnet>.ts.net/<capability-token>/mcp
```

The hostname persists because:

- `tsnet.Server.Hostname` remains fixed;
- `tsnet.Server.Dir` points to a persistent app-data directory;
- the node is non-ephemeral;
- the sidecar does not delete its node state during Stop.

A user-visible **Reset connection identity** action is the only normal UI flow that deletes/rotates the local Tailscale identity and capability secret.

## 5. Public endpoint protection

### 5.1 Why a protection layer is mandatory

Tailscale Funnel is public internet ingress. The current MCP server uses no application-layer authentication because it was designed behind OpenAI Secure MCP Tunnel. Publishing the raw current `/mcp` route would expose local read/write tools to anyone who can reach the URL.

### 5.2 Capability URL for this milestone

For this milestone, protect the public route with a high-entropy persistent capability token embedded as one URL path component.

Requirements:

- Generate 32 random bytes using the OS CSPRNG and encode as URL-safe base64 without padding or lowercase hex.
- Minimum entropy: 256 bits.
- Do not accept the token through query parameters.
- Do not expose the token in process argv.
- Do not print the token in backend debug logs.
- Public requests that do not match the exact capability path return generic `404 Not Found`, not `401`, to avoid advertising an MCP service.
- The sidecar rewrites the accepted public MCP path to local `/mcp`.
- The same capability prefix protects the end-to-end health route.
- Rotate only through an explicit Security action.

This is a capability URL, therefore the URL itself is a credential. UI copy must say: **Anyone with this URL may attempt to use this MCP connection. Keep it private.**

OAuth 2.1 is intentionally outside this implementation milestone; it can replace the capability route later without changing the local workspace security core.

### 5.3 Secret storage

Use Windows DPAPI from Rust for the capability token at rest.

- Persist encrypted token data in app-data settings.
- Decrypt only when starting the sidecar or rendering the copyable endpoint.
- Pass the capability token to the Go sidecar via environment variable, never command-line flags.

Tailscale node state remains in a dedicated app-data directory managed by tsnet.

## 6. Connection lifecycle

Define these states in the Rust backend and frontend contract:

```text
Stopped
StartingServer
StartingTunnel
NeedsLogin
NeedsFunnelPermission
Verifying
Online
Degraded
Stopping
Error
```

### 6.1 Start

When the user presses Start:

1. Confirm at least one workspace has read access; otherwise keep connection stopped and show an actionable empty-state message.
2. Confirm local MCP server is running and obtain its current loopback port.
3. Ensure a capability token exists; generate and persist one if missing.
4. Ensure the tsnet state directory exists.
5. Spawn `tailscale-funnel-sidecar.exe` with:
   - local MCP target port in argv only if considered non-secret;
   - fixed hostname;
   - state directory;
   - capability token via environment variable.
6. Parse newline-delimited JSON events from sidecar stdout.
7. If sidecar reports `auth_required`, enter `NeedsLogin`; UI offers **Sign in to Tailscale** and opens the returned Tailscale login URL.
8. Once authenticated, sidecar calls `ListenFunnel("tcp", ":443", tsnet.FunnelOnly())`.
9. If Funnel is not enabled/authorized for the tailnet, enter `NeedsFunnelPermission` with clear one-time setup instructions and a Retry action.
10. Sidecar reports `funnel_listening` with FQDN.
11. Rust composes the public capability endpoint.
12. Enter `Verifying`; perform an HTTPS request to the capability-protected public health route.
13. Only after expected health response is received, enter `Online`.

### 6.2 Stop

When the user presses Stop:

1. Enter `Stopping`.
2. Ask the sidecar to shut down gracefully or terminate the tracked process.
3. Close the Funnel listener and tsnet server.
4. Keep the tsnet state directory intact.
5. Keep the capability secret intact.
6. Return to `Stopped`.

Stopping never resets the public identity.

### 6.3 App exit/crash resilience

Normal exit must terminate the sidecar.

On Windows, bind the sidecar to a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so forceful parent termination does not leave an orphan public Funnel process.

On next app start, stale PID metadata is ignored; the app derives truth from the current managed process and fresh sidecar events.

## 7. Sidecar IPC contract

Use newline-delimited JSON on stdout for machine-readable events. Human/debug logs go to stderr and must never contain the capability token.

Events:

```json
{"type":"starting"}
{"type":"auth_required","url":"https://login.tailscale.com/a/..."}
{"type":"authenticated"}
{"type":"funnel_listening","fqdn":"local-workspace-mcp.example.ts.net"}
{"type":"health_ready"}
{"type":"error","code":"FUNNEL_NOT_ALLOWED","message":"..."}
{"type":"stopped"}
```

Rust owns the user-facing connection state and maps sidecar events to `ConnectionState`.

Sidecar startup arguments:

```text
--target http://127.0.0.1:<port>
--state-dir <app-data>/tailscale
--hostname local-workspace-mcp
```

Environment:

```text
MCP_CAPABILITY_TOKEN=<secret>
```

## 8. Sidecar HTTP routing

The Funnel listener is TLS-wrapped by tsnet on port 443.

Routes:

```text
GET  /<token>/health          -> 204 + X-MCP-Bridge-Health: ok
ANY  /<token>/mcp             -> proxy to http://127.0.0.1:<port>/mcp
ANY  /<token>/mcp/...         -> proxy preserving suffix when protocol requires it
ANY  everything else          -> 404
```

Proxy requirements:

- preserve MCP-required request headers except hop-by-hop headers;
- preserve streaming/SSE behavior;
- disable response buffering;
- support long-lived Streamable HTTP sessions;
- set conservative HTTP server timeouts that do not terminate valid MCP streams;
- never proxy to arbitrary targets; target is loopback-only and provided by the parent app.

## 9. Rust connection manager

Replace tunnel-specific lifecycle code with a Tailscale-focused connection manager.

Recommended modules:

```text
src-tauri/src/connection/
  mod.rs
  manager.rs
  state.rs
  sidecar.rs
  health.rs
  secrets.rs
  windows_job.rs
```

Responsibilities:

- `state.rs`: serializable state enum + status payload.
- `sidecar.rs`: locate/spawn sidecar, parse stdout JSON, capture stderr diagnostics.
- `manager.rs`: state machine, start/stop/retry/reset orchestration.
- `health.rs`: public end-to-end health verification.
- `secrets.rs`: DPAPI capability token storage/rotation.
- `windows_job.rs`: child lifecycle binding.

The existing filesystem core remains in `crates/core`.

### 9.1 Workspace execution subsystem

Add execution as a third workspace capability alongside Read and Write:

```text
Read      ON/OFF
Write     ON/OFF
Execute   ON/OFF  (default OFF)
```

Persist these additional workspace fields:

```text
can_execute: bool
execution_mode: "approved_commands" | "full_powershell"
```

`execution_mode` defaults to `approved_commands`. Turning Execute OFF blocks every process-execution MCP tool regardless of mode.

The MCP surface gains two tools:

```text
run_command(workspace_id, program, args[], cwd?, timeout_ms?)
run_powershell(workspace_id, script, cwd?, timeout_ms?)
```

`run_command` launches an executable directly without `cmd.exe` or PowerShell parsing. Requirements:

- `cwd` is workspace-relative and must pass the same canonical path-confinement checks as file tools;
- executable basename must match the configured approved-command allowlist;
- arguments are passed as an argv array, never concatenated into a shell command string;
- default timeout 60 seconds; configurable hard maximum 10 minutes;
- stdout and stderr are captured separately and bounded to 512 KiB each, with explicit truncation flags;
- child process tree is attached to a Windows Job Object so timeout/Stop kills descendants;
- inherited environment is sanitized and must not expose MCP capability tokens, Tailscale auth URLs, legacy API keys, or other app-owned secrets;
- audit records command basename, safe args metadata, relative cwd, exit code, duration, timeout/truncation flags, but never captures environment secrets or full script contents.

Initial approved developer commands should be explicit product configuration rather than arbitrary PATH execution. A sensible default set is `git`, `cargo`, `rustc`, `npm`, `npx`, `node`, `pnpm`, `yarn`, `python`, `python3`, `pip`, `go`, and `dotnet`. The UI may allow users to edit this list in Advanced settings.

`run_powershell` is available only when `can_execute == true` and `execution_mode == "full_powershell"`. It launches `pwsh.exe` when available, otherwise Windows PowerShell, with `-NoLogo -NoProfile -NonInteractive`; the script is supplied through stdin so it is not exposed in argv. Full PowerShell is explicitly **not** confined to the workspace by `cwd`; it can potentially access the wider user/system environment according to the Windows account's permissions. The UI must warn about this before enabling the mode.

Stable execution errors:

```text
EXECUTE_DISABLED
COMMAND_NOT_ALLOWED
POWERSHELL_DISABLED
INVALID_WORKING_DIRECTORY
COMMAND_TIMEOUT
EXECUTION_FAILED
```

A future optional **Strict Isolation** mode may use Windows Sandbox with only the workspace mapped into the VM and networking disabled by default. It is not part of this milestone because Windows Sandbox is edition/virtualization dependent and adds significant startup overhead.

## 10. UI information architecture

Replace the current one-screen MVP with a compact desktop shell.

```text
Sidebar
  Dashboard
  Workspaces
  Activity
  Security
  Settings
```

### 10.1 Visual direction

- Native-desktop feel, restrained and professional.
- Clear hierarchy rather than decorative cards everywhere.
- Neutral surfaces, subtle borders, consistent spacing/radii.
- One accent color used for primary actions/status emphasis.
- Status uses icon + label, never color alone.
- Avoid excessive gradients, oversized marketing headers, and dashboard clutter.
- Use reusable CSS classes/components; remove most inline styles.
- Provide keyboard-visible focus states and accessible labels.

### 10.2 Dashboard

Primary content:

- App/service name and compact connection summary.
- Large but not oversized connection card.
- Current state.
- Primary Start/Stop button.
- Public MCP endpoint when Online.
- Copy endpoint action.
- Connection privacy warning.
- Workspace summary: count, read-enabled count, write-enabled count.
- Security summary.
- Five most recent activity items.

State-specific copy examples:

```text
Stopped: "Your local MCP connection is offline."
StartingTunnel: "Starting secure Tailscale connection…"
NeedsLogin: "Sign in to Tailscale to connect this app."
NeedsFunnelPermission: "Funnel needs one-time approval for this tailnet."
Verifying: "Verifying the public MCP endpoint…"
Online: "Ready for ChatGPT."
Degraded: "Connection started, but end-to-end verification failed."
```

### 10.3 Workspaces

Each workspace row/card shows:

- name;
- root path;
- Read toggle;
- Write toggle;
- Execute toggle;
- execution mode summary (`Approved commands` or `Full PowerShell`) when Execute is enabled;
- enabled/disabled summary;
- overflow actions for remove and advanced execution settings.

Rules:

- Disabling read while write is enabled must also disable write, because write capability cannot exist without read in the current core model.
- Enabling write when read is disabled first enables read.
- Execute is independent from Read/Write and defaults OFF for every newly added workspace.
- Turning Execute ON defaults to `Approved commands`; it must not silently enable Full PowerShell.
- Enabling `Full PowerShell` requires an explicit confirmation that PowerShell is not restricted to the workspace merely because its working directory is there.
- Removing requires confirmation and explicitly says disk files are not deleted.
- Add Workspace uses native folder picker.

### 10.4 Activity

Replace the cramped audit table with activity rows and a detail panel.

List shows:

- local time;
- source (`ChatGPT`/`App`);
- operation;
- workspace name if resolvable;
- decision badge;
- path when available.

Filters:

- All / Allowed / Blocked / Errors;
- operation search text.

Selecting an event opens detail with timestamp, tool/op, workspace, decision, stable error code, path, and safe detail JSON. Execution events additionally show command basename, relative working directory, exit code, duration, timeout state, and output-truncation flags. Do not store or show complete PowerShell script bodies in audit history.

### 10.5 Security

Show:

- Global access status + Pause/Resume.
- Public endpoint protection: Capability URL active.
- Sensitive-file deny list summary.
- Path confinement active.
- Optimistic concurrency active.
- Write-enabled workspace count.
- Execute-enabled workspace count.
- Full-PowerShell-enabled workspace count with a high-risk warning when non-zero.
- Explicit note that Approved Commands/Full PowerShell are process-execution permissions, not filesystem sandboxes.
- **Rotate connection URL** destructive action.

Rotating the connection URL:

- generates a new capability token;
- invalidates the old public URL immediately after restart/reload of sidecar routing;
- preserves the Tailscale hostname;
- requires confirmation because the ChatGPT connection must be updated.

### 10.6 Settings

Sections:

**General**
- Start connection automatically on app launch: default OFF.
- Minimize to tray: deferred; do not add unless tray is implemented in same milestone.

**Connection**
- Hostname (advanced; default `local-workspace-mcp`). Changing it requires connection identity reset/re-enrollment warning.
- Current Tailscale FQDN read-only.
- Tailscale state location read-only.
- Retry connection.
- Reset Tailscale identity.

**Execution**
- Read-only list of approved developer commands for this milestone; custom allowlist editing is deferred.
- Default command timeout: 60 seconds.
- Hard maximum timeout: 10 minutes.
- Explanation that Approved Commands can still execute project code (for example npm scripts, Cargo build scripts, Git hooks) and therefore Execute must remain OFF unless needed.
- Future Strict Isolation mode status: not available in this milestone.

**Advanced**
- Local MCP port read-only while running.
- Copy diagnostics (must redact capability token and execution environment secrets).

**About**
- version;
- links to MCP/Tailscale docs;
- licenses.

## 11. Onboarding

First launch should not show a wall of settings.

Flow:

1. Welcome — explain that the app lets ChatGPT access only approved folders.
2. Add first workspace.
3. Review default permissions: Read ON, Write OFF.
4. Press Start.
5. If `NeedsLogin`, open Tailscale sign-in.
6. If Funnel approval is required, show one-time instructions and Retry.
7. On Online, show endpoint with Copy button and short ChatGPT connection instructions.

After onboarding is complete, launch directly into Dashboard.

## 12. Audit improvements required for the new Activity UI

The current MCP audit pipeline often records `path = NULL` and empty `detail`. Improve operation audit context without logging file contents or secrets.

Allowed audit detail examples:

```json
{"bytes":1234,"truncated":false}
{"matches":12,"truncated":false}
{"bytes_changed":18}
```

Never audit:

- file content;
- search result lines containing sensitive data beyond the already-returned safe result contract;
- capability token;
- Tailscale auth URL;
- API/auth keys.

## 13. Reliability hardening included in this milestone

### 13.1 Windows atomic replacement

The project is Windows-first. Replace the current `std::fs::rename(temp, existing_target)` assumption for `apply_patch` with a Windows-safe atomic replacement implementation and tests. Existing-file patching must be proven on Windows.

### 13.2 Sidecar orphan prevention

Use Windows Job Object kill-on-close.

### 13.3 Verified Online status

Never map successful process spawn directly to Online.

### 13.4 Settings migration

Existing OpenAI tunnel settings must not crash startup.

Migration behavior:

- leave legacy fields readable for one release;
- do not auto-start the old `tunnel-client`;
- new connection manager uses Tailscale only;
- after successful migration, legacy secrets should be removed from the new settings representation;
- old binaries can be removed from packaging once Tailscale E2E is green.

## 14. Build/package changes

Add a Go module for the sidecar:

```text
sidecars/tailscale-funnel/
  go.mod
  go.sum
  main.go
  internal/bridge/...
```

Build output follows Tauri externalBin naming:

```text
src-tauri/binaries/tailscale-funnel-sidecar-x86_64-pc-windows-msvc.exe
```

Release build must fail clearly if the expected sidecar binary is missing.

Track third-party license attribution for Tailscale/Go dependencies as required by their licenses.

## 15. Testing strategy

### Rust unit tests

- connection state transitions;
- secret generation and DPAPI round-trip on Windows;
- stdout event parsing;
- URL composition;
- settings migration;
- audit enrichment;
- Windows atomic replacement;
- workspace Execute capability migration/defaults;
- executable allowlist matching and direct argv execution;
- working-directory confinement;
- timeout/output bounds and process-tree termination;
- Full PowerShell gating and stdin script delivery;
- execution audit redaction.

### Go unit tests

- capability route exact match;
- non-matching paths return 404;
- health route;
- reverse-proxy path rewrite;
- token never appears in logs;
- sidecar event JSON format.

### Rust integration tests

Use a stub sidecar executable/script to deterministically emit events:

- first-run login flow;
- online flow;
- Funnel permission error;
- crash/restart;
- stop during startup;
- health verification failure -> Degraded/Error.

### MCP integration tests

Keep existing real rmcp/axum loopback tests and add coverage for audit path/detail.

### Manual Windows E2E gate

1. Clean app state.
2. Add workspace.
3. Start.
4. Sign in to Tailscale.
5. Approve Funnel if required.
6. Reach Online.
7. Add URL to ChatGPT.
8. List/read workspace.
9. Enable write and patch a file.
10. Stop app; confirm endpoint is offline.
11. Relaunch and Start; confirm same FQDN and capability URL.
12. Confirm old URL fails after Rotate connection URL.
13. Force-kill parent app and verify no sidecar remains.

## 16. Acceptance criteria

The milestone is complete only when all are true:

- The main UI no longer requires tunnel ID/API key entry.
- First-run Tailscale authentication is understandable and recoverable.
- Normal connection requires one Start click.
- Same app state produces the same `*.ts.net` hostname after restart.
- Same capability token produces the same full MCP URL after restart.
- Public raw `/mcp` is not reachable.
- Incorrect capability paths return 404.
- Online is shown only after public health verification succeeds.
- Stop closes public access without deleting Tailscale identity.
- No sidecar remains after normal exit or parent-process death on Windows.
- Read/write workspace behavior remains enforced by the existing fail-closed pipeline.
- Execute is a separate per-workspace permission, defaults OFF, and is audited.
- `run_command` rejects non-allowlisted executables and confines only the requested working directory to the workspace.
- `run_powershell` is unavailable unless Full PowerShell mode was explicitly enabled for that workspace.
- Full PowerShell UI copy clearly states it is not filesystem-confined by `cwd`.
- Timed-out executions terminate their process tree and bounded stdout/stderr results report truncation.
- Sensitive-file and path-confinement protections still pass.
- Existing-file `apply_patch` is verified working on Windows.
- Dashboard, Workspaces, Activity, Security, and Settings screens are implemented with coherent reusable styling.
- `npm run build`, Rust tests, Go tests, and the manual Funnel/ChatGPT E2E gate all pass.

## 17. Explicit non-goals for this milestone

To keep the implementation coherent, do not add these in the same milestone:

- OAuth 2.1 authorization server for MCP.
- Cloudflare/ngrok providers.
- Dedicated Git MCP tools beyond invoking approved `git` commands through the execution subsystem.
- Strict VM/container isolation for arbitrary commands (future Windows Sandbox mode).
- delete/move/rename filesystem tools.
- mobile support.
- macOS/Linux packaging.
- multi-user accounts inside the desktop app.
- remote workspace management.

These can be separate future designs after the Tailscale Funnel release is stable.
