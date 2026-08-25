# Tailscale Funnel E2E Test Report (Plan Task 23)

Date: 2026-08-25 (updated same day after security-hardening pass) · Branch: `feat/tailscale-funnel` · Build: release bundle from `8cd312a`

## Status: IN PROGRESS — NOT production-ready

**The gate below has NOT been met.** No scenario in this report has been
executed against a real Tailscale tailnet or a real ChatGPT connector yet.
Do not claim the Funnel milestone complete until every checkbox passes or
carries a documented external prerequisite.

### What IS already proven (automated, no network)

- `cargo test --workspace` green: 247 tests, including
  - 6 connection-lifecycle integration tests driving the REAL sidecar spawn +
    NDJSON parse pipeline against a fake sidecar binary;
  - 17 MCP integration tests over the live axum/rmcp server: pipeline denies
    (pause, capability, confinement, sensitive deny-list), audit-row
    invariants (exactly one row per call, fail-closed on audit-write failure),
    bounded execution (timeout kills the tree, truncation metadata);
  - settings v3 round-trip incl. theme, tray, autostart and unrestricted flags.
- Release build + NSIS installer packaging succeeds; app boots to the new
  shadcn/Tailwind UI (light and dark verified visually), tray menu, hidden
  child consoles and start-to-tray verified by manual smoke.
- Still unproven: everything requiring the real tailnet (login, funnel
  publish, public reachability, ChatGPT connector) — the scenarios below.

### New manual checks added with the 2026-08-25 hardening pass

- [ ] Unrestricted access: enabling requires typing UNRESTRICTED; red banner
      shows; with it ON a write to a `.env`-style file SUCCEEDS and its audit
      row carries `"unrestricted": true`; Pause still blocks calls; turning
      the switch off restores SENSITIVE_FILE_BLOCKED.
- [ ] list_workspaces from ChatGPT shows ids/names only (no absolute paths)
      and is denied with ACCESS_PAUSED while paused.
- [ ] Dashboard "open endpoint" button opens `https://<fqdn>.ts.net` in the
      browser (opener capability now allows `*.ts.net`).
- [ ] Quitting via tray menu stops the sidecar gracefully (no orphan after
      normal exit; Job Object still covers force-kill per Step 10).
- [ ] App boots hidden to tray when Settings → "Start minimized to the system
      tray" is on; window reappears via tray or a second launch attempt.

## Prerequisites (done before scenarios)
- [x] Go sidecar built (`npm run build:sidecar` → go test pass, binary in `src-tauri/binaries/`)
- [ ] Release app launched from `src-tauri/target/release/local-workspace-mcp-manager.exe`
      (or NSIS installer under `target/release/bundle/nsis/`)
- [ ] Clean app-specific Tailscale state dir before first run

## Scenarios

### Step 1 — Clean state
Back up then remove only the app-specific test state directory (shown in
Settings → Connection → "Tailscale state location"). Do NOT touch other
Tailscale user data.

Result: ☐

### Step 2 — Temporary workspace
Add a temp folder with non-sensitive files via Add folder. Enable write later
when needed.

Result: ☐

### Step 3 — Start + login
Press Start; complete Tailscale login in the browser when the auth URL opens.
Expected transition: NeedsLogin → Starting/Authenticated → Funnel listening →
Verifying → Online. If Funnel approval is disabled, UI must show
NeedsFunnelPermission; enable Funnel once in the Tailscale admin console,
press Retry.

Result: ☐

### Step 4 — Endpoint into ChatGPT
Copy endpoint URL from Security/Dashboard and paste into ChatGPT custom MCP
connector config. Tool scan/list must succeed.

Result: ☐

### Step 5 — MCP read scenario
Via ChatGPT: list workspaces / list files of test workspace / read a test file /
search a unique string. Each call must appear in Activity.

Result: ☐

### Step 6 — MCP write scenario
Enable write on workspace, read file revision, apply patch via ChatGPT, verify
disk content changed exactly once, second identical write is rejected or bumps
revision per spec.

Result: ☐

### Step 6A — Execution scenarios
With Execute OFF: run_command → EXECUTE_DISABLED.
Enable Approved-commands mode:
- `run_command("git", ["--version"])` → result with exit code/output
- `run_command("cmd.exe", ["/C", "echo hi"])` → COMMAND_NOT_ALLOWED
- `run_powershell(...)` → POWERSHELL_DISABLED
- `cwd=".."` → PATH_OUTSIDE_WORKSPACE
Switch to Full PowerShell after warning confirm:
- `Write-Output "ok"`, `Get-Location` succeed with workspace as initial cwd.
Short sleeper command → timeout terminates tree; Activity records timeout/
truncation metadata without contents. UI warning remains visible in
Security/workspace details.

Result: ☐

### Step 7 — Security boundaries
From an external client (phone on tailnet or curl):
- raw `https://FQDN/mcp` → 404
- wrong capability token → 404
- read `.env` → SENSITIVE_FILE_BLOCKED
- `../` traversal → PATH_OUTSIDE_WORKSPACE
- Pause in app → next tool call → ACCESS_PAUSED

Result: ☐

### Step 8 — Stable URL across restart
Record endpoint, Stop, exit app, relaunch, Start. Same FQDN + same capability
endpoint.

Result: ☐

### Step 9 — Rotation
Rotate connection URL → old URL 404, new URL works, FQDN unchanged.

Result: ☐

### Step 10 — Force-kill cleanup
Start Online, force-kill Tauri parent in Task Manager → no
`tailscale-funnel-sidecar` process remains (`tasklist | findstr funnel`).

Result: ☐

## Gate decision
Task 24 (legacy removal) proceeds only if every scenario above passes or has
an explicitly documented non-code external prerequisite.
