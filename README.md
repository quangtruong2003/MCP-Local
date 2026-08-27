# Local Workspace MCP Manager

A Windows desktop app that lets **ChatGPT Web** (Developer Mode) securely read, search, list, and edit files inside folders you explicitly approve — and optionally run approved developer commands. Built as a single-process Rust/Tauri application embedding an MCP server (Streamable HTTP on `127.0.0.1`) that is published to ChatGPT through **Tailscale Funnel**.

There is exactly one MCP doorway to your filesystem: a fail-closed policy pipeline. Every MCP tool call goes through it. The desktop UI is a separate administrative control plane for workspace permissions, connection settings, and lifecycle; UI audit rows are operational history rather than the MCP pipeline's fail-closed response gate.

## How the connection works

- The MCP server binds **loopback only** (`127.0.0.1:<ephemeral port>`).
- A bundled **Tailscale Funnel sidecar** publishes that loopback server to the public internet at `https://<your-hostname>.ts.net` (Funnel requires a one-time tailnet admin enablement: HTTPS certificates + the funnel node attribute).
- The public URL embeds a **secret capability token**. Funnel is *public ingress* — the secret URL is what protects the endpoint, so treat it like a password: anyone who obtains it can call your MCP connection. Rotate it any time from **Security → Rotate connection URL**.

## Tech stack

- **Shell**: Tauri 2 (Rust) with a system tray (open, start/stop tunnel, pause MCP access, quit), close-to-tray, launch-at-login, start-minimized-to-tray, and single-instance enforcement
- **UI**: React 18 + TypeScript (strict), styled with Tailwind CSS v4 and vendored shadcn/ui primitives (new-york/zinc, 2px-radius "Quiet Console" tokens); toasts via sonner; bundled Inter Variable / JetBrains Mono Variable fonts; Light/Dark/System theme persisted in settings
- **Core pipeline**: `crates/core` — the fail-closed policy pipeline (pause → workspace → capability → confinement → sensitive-file deny list → execution) with an append-only SQLite audit log

## Prerequisites

- Windows 10/11
- Rust toolchain with the MSVC target (`rustup default stable-x86_64-pc-windows-msvc`) and Visual Studio Build Tools (C++ workload)
- Node.js (LTS) + npm
- A Tailscale account (free tier works); a tailnet admin must enable Funnel once (HTTPS certificates under DNS settings + the funnel node attribute under Access controls)

## Build & run

```powershell
npm install

# Development (hot-reload UI):
npx tauri dev

# Release build (produces an NSIS installer):
npx tauri build
```

Artifacts land under `target/release/` (executable) and `target/release/bundle/nsis/` (installer).

### Sidecar binaries

The Tailscale Funnel sidecar (Go, in `sidecars/tailscale-funnel`) is built by the packaging step (`npm run build:sidecar` → `scripts/build-tailscale-sidecar.ps1`) and is the only connection sidecar bundled as `binaries/tailscale-funnel-sidecar`. The retired OpenAI/Cloudflare tunnel runtime is no longer compiled or packaged; legacy settings parsing remains only for upgrade compatibility.

## First-run setup

1. Launch the app and complete the onboarding wizard: add at least one **Workspace** (Read starts on, Write/Execute off), then sign in to Tailscale when prompted.
2. Press **Start** on the Dashboard. The app starts the local MCP server, spawns the Funnel sidecar, and walks you through any one-time tailnet approvals.
3. When the card turns **Online**, copy the **secret capability URL** into ChatGPT (Developer Mode → create a connector of type MCP / remote endpoint) and keep it private.

Everyday behavior is configurable in **Settings**:

- **General**: auto-start the tunnel when the app opens, start with Windows, start minimized to the tray, keep running in the tray when the window closes
- **Appearance**: Light / Dark / System theme
- **Agent**: Autonomous / Conservative guidance, explicit screen-capture permission, capture scope, and visual-verification preference
- **Connection**: the Funnel hostname label (changes require a Tailscale identity reset)

The system tray mirrors the essentials: open the window, start/stop the tunnel, pause/resume MCP access, quit.

## MCP tool surface

Forty-three tools are exposed. `get_agent_policy` reports live Agent/Privacy state; `list_workspaces` returns approved workspace ids without absolute paths. The nine workspace-scoped file/execution tools are `list_directory`, `find_files`, `search_text`, `read_file`, `read_files` (≤8 per call), `create_file` (≤10 MB), `apply_patch`, `run_command` (approved basenames: git, cargo, rustc, npm, npx, node, pnpm, yarn, python, python3, pip, go, dotnet), and `run_powershell` (requires the workspace's Full PowerShell mode unless unrestricted access is enabled). `list_capture_targets` and `capture_screen` provide permission-gated visual evidence. Read tools return bounded results (`read_file` returns a ≤256 KB window with a truncation flag) and carry a `revision` (sha256) that `apply_patch` requires as `base_revision`.

`capture_screen` returns one in-memory PNG image block plus dimensions, encoded byte count, and SHA-256 metadata. Screen capture is off by default, has its own permission independent of unrestricted filesystem/execution access, can be limited to windows only, and is always blocked by Pause.

## Browser automation

The app can drive a real browser on the agent's behalf — navigate, click, fill, read the accessibility snapshot/DOM, capture screenshots, collect console and network diagnostics, record traces, and upload/download files — through the same fail-closed policy pipeline as the file and command tools. A dedicated family of `browser_*` tools covers interaction, inspection, visual evidence, debugging, file transfer, and batched steps.

- **On by default, one toggle**: Browser Automation defaults **ON** but can be disabled with a single Settings toggle. When off, every browser tool fails closed with a clear blocker.
- **Bundled Chromium + dedicated profile**: the defaults are bundled Chromium and a dedicated **Local Workspaces** browser profile stored inside the app's own data folder — completely separate from your personal browser profile, bookmarks, passwords, and history.
- **Background mode (default)**: runs in the background and does **not** open a normal browser window.
- **Visible Debug (opt-in)**: can show a Chromium-family browser window plus DevTools for step-by-step debugging.
- **Screenshots are page-renderer captures**: browser screenshots come from the page renderer and are independent of the OS Screen Capture permission.
- **Files stay confined**: uploads and downloads remain sandbox/workspace-confined; downloads land in the session artifact store and reach a workspace only through an explicit, policy-checked export step.
- **Pause wins**: Pause blocks browser automation immediately, just like every other MCP tool.

**Installer size impact**: the release stages a self-contained browser runtime — bundled Node, Playwright MCP, and matching Chromium and Firefox binaries (a **~1.1 GiB** staged runtime) — so the installer and installed footprint are substantially larger than a non-browser build.

## Agent Skill and fallback prompts

The repository ships a first-class Agent Skill at `plugin/skills/local-workspace-agent/`. `npm run package:skill` produces `dist/local-workspace-agent.zip`, which can be uploaded in ChatGPT from **Plugins → Skills → Create → Upload from your computer** when skill uploading is enabled for the account/workspace. The Skill teaches the `DISCOVER → PLAN → INSPECT → ACT → VERIFY → RECOVER → REPORT` workflow and composes the Local Workspaces MCP app; it does not grant authority.

The MCP server also publishes the same operating contract as fallback initialization instructions and exposes six prompts: `agent-task`, `explore-project`, `implement-change`, `debug-fix`, `debug-web`, and `review-verify`. This keeps compatible MCP hosts useful even when they do not load the first-class Skill.

Pause, workspace capabilities, path confinement, sensitive-file policy, execution policy, screen-capture authorization, and fail-closed MCP audit remain backend-enforced. The local MCP server intentionally stays on `127.0.0.1:<ephemeral>/mcp`; Tailscale Funnel publishes the secret capability endpoint used by the ChatGPT app.

Local build/package commands produce two independently testable artifacts: the Local Workspaces MCP app endpoint and `dist/local-workspace-agent.zip`. Creating, sharing, or publishing a workspace plugin that combines the app and Skill is a separate ChatGPT workspace operation; this repository does not invent or emit an undocumented plugin manifest. See `docs/plugin/CHATGPT_INSTALL.md` for the handoff flow.

## Security model

- **Loopback-only binding**: the MCP server listens strictly on `127.0.0.1` — never on all interfaces. The only external exposure is the sidecar's outbound HTTPS funnel.
- **Secret capability URL**: the public endpoint embeds an unguessable capability token; there is no login flow on the endpoint. Rotate it from the Security page if it leaks.
- **Pause kill switch**: one toggle (UI, tray) suspends all MCP tool calls (`ACCESS_PAUSED`), including `list_workspaces`; UI operations keep working. Pause wins even in unrestricted mode.
- **Fail-closed pipeline**: every tool invocation passes one pipeline — pause check, workspace lookup, capability gates (`READ_DISABLED`/`WRITE_DISABLED`/`EXECUTE_DISABLED`), path confinement (canonicalized, component-wise containment; symlinks/junctions cannot escape), sensitive-file deny-list, execution policy — then writes exactly one audit row. Any pipeline failure short-circuits to a structured error, and an unrecordable audit row fails the call.
- **Sensitive-file deny-list**: `.env*`, `*.pem`, `*.key`, `id_rsa*`, `.git/config`, `credentials*`, `*.pfx`, `*.p12`, `secrets.*`, and all of `node_modules/**` are blocked at any depth, for reads and writes alike. Overriding requires unrestricted mode (see below) or a code change.
- **Patch-first editing**: edits are anchored string replacements (`apply_patch`), not whole-file writes; every edit carries a `base_revision` (SHA-256 of content as last seen) and fails with `FILE_CHANGED_SINCE_READ` if the file changed externally. New files are written atomically via temp-file + rename.
- **Append-only audit log**: every MCP request writes exactly one audit row (allow / deny / error) before its response is sent; rows can never be updated or deleted. Rows produced in unrestricted mode are marked. The latest 100 events are visible in the UI.
- **Bounded execution**: commands run without a shell (no quoting/parsing surface), inside a Windows Job Object (timeout kills the whole process tree), with sanitized environment, 60 s default / 10 min max timeout, and 512 KiB captured per output stream. PowerShell scripts travel through stdin only — never argv, audit detail, or diagnostics — and child console windows are never shown.
- **Unrestricted access (opt-in)**: a Security-page switch, gated behind a typed confirmation and a persistent red banner. It relaxes capability gates, the sensitive-file deny list, the approved-command allowlist, and the Full-PowerShell workspace gate — for every workspace. The folder boundary (paths still resolve inside registered workspaces), the pause kill switch, audit recording, and screen-capture permission remain in force. Leave it off unless you fully trust your conversations.
- **Screen capture (separate opt-in)**: screen capture defaults off and unrestricted access never enables it. Pause always wins. Capture is single-shot only; the capture path returns PNG bytes in memory and does not persist screenshots. Audit rows retain safe metadata such as dimensions, byte count, target id/kind, and SHA-256, never image pixels/base64 or window titles.
- **Secret handling**: the capability token never appears in UI read-back paths, command lines, or diagnostics. Connection secrets live in a DPAPI-protected store under the current user's LocalAppData; `settings.json` holds only non-secret preferences.
- **Hardened webview**: a non-null Content-Security-Policy (`default-src 'self'`, IPC-restricted `connect-src`, inline styles allowed for Radix positioning); the opener plugin allowlists only `login.tailscale.com`, `tailscale.com`, and `*.ts.net` URLs.

### Known limitations (honesty notes)

- Working-directory confinement is not a Windows security sandbox: an executed program can still open paths outside the workspace with your account's permissions.
- Funnel is public ingress; the endpoint's only protection is the secrecy of the capability URL. Rotate it if in doubt.
- Real Tailscale-tailnet + ChatGPT connector acceptance remains environment-dependent and must still be run before calling a release production-certified; the automated local lifecycle/MCP suites do not substitute for that external check.

## Error codes

Tool errors surface as structured results `{ code, message, detail? }` with these stable codes:

| Code | Meaning |
|---|---|
| `ACCESS_PAUSED` | kill switch engaged |
| `WORKSPACE_NOT_FOUND` | unknown workspace_id |
| `READ_DISABLED` / `WRITE_DISABLED` | capability off |
| `EXECUTE_DISABLED` / `POWERSHELL_DISABLED` | execution off / workspace not in Full-PowerShell mode |
| `COMMAND_NOT_ALLOWED` | program not on the approved list (or a path was given) |
| `PATH_OUTSIDE_WORKSPACE` | confinement violation (audited with detail) |
| `SENSITIVE_FILE_BLOCKED` | deny-list hit |
| `FILE_NOT_FOUND` | target missing |
| `FILE_EXISTS` | create collision |
| `INVALID_PATCH` | anchor missing/ambiguous |
| `FILE_CHANGED_SINCE_READ` | revision mismatch |
| `RESULT_TOO_LARGE` | bounds exceeded without truncation possible |
| `INVALID_ARGUMENT` | malformed client input (bad regex/glob, >8 read_files paths, invalid capture size) |
| `SCREEN_CAPTURE_DISABLED` | screen capture has not been enabled by the user |
| `SCREEN_CAPTURE_UNSUPPORTED` | capture is unavailable on this platform |
| `CAPTURE_TARGET_NOT_FOUND` | target disappeared or is outside the allowed capture scope |
| `SCREEN_CAPTURE_FAILED` | target enumeration, capture, resize, or PNG encoding failed |
| `IO_ERROR` | filesystem failure (OS message included) |

## Development

```powershell
npx tsc --noEmit        # frontend type gate
npm run build           # frontend production build
cargo test --workspace  # 240+ Rust unit + integration tests
```

The integration suites drive the real sidecar spawn/parse pipeline against a fake sidecar fixture — no Tailscale network needed.
