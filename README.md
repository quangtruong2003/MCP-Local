# Local Workspace MCP Manager

A Windows desktop app that lets **ChatGPT Web** (Developer Mode) securely read, search, list, and edit files inside folders you explicitly approve. Built as a single-process Rust/Tauri application embedding an MCP server (Streamable HTTP on `127.0.0.1`) that is bridged to ChatGPT via OpenAI's official Secure MCP Tunnel (`tunnel-client` sidecar).

There is exactly one doorway to your filesystem: a fail-closed policy pipeline. Both the MCP tools and the app's own UI go through it.

## Prerequisites

- Windows 10/11
- Rust toolchain with the MSVC target (`rustup default stable-x86_64-pc-windows-msvc`) and Visual Studio Build Tools (C++ workload)
- Node.js (LTS) + npm

## Build & run

```powershell
npm install

# Development (hot-reload UI):
npx tauri dev

# Release build (produces an NSIS installer):
npx tauri build
```

Artifacts land under `src-tauri/target/release/` (executable) and `src-tauri/target/release/bundle/nsis/` (installer).

## Sidecar binaries (required)

Two sidecar executables are **gitignored and not in this repository**. The release build and bundling step fails without them.

1. Download **tunnel-client v0.0.12** from <https://github.com/openai/tunnel-client/releases> (Windows amd64 archive). Verify the SHA256 of the downloaded archive against the release's `SHA256SUMS.txt` before extracting:

   ```powershell
   Get-FileHash .\tunnel-client-v0.0.12-windows-amd64.zip -Algorithm SHA256
   ```

2. Extract both binaries from the archive into `src-tauri/binaries/`, renamed with Tauri's target-triple suffix:

   ```
   src-tauri/binaries/tunnel-client-x86_64-pc-windows-msvc.exe
   src-tauri/binaries/cloudflared-x86_64-pc-windows-msvc.exe
   ```

3. Place `cloudflared` beside `tunnel-client` in the same directory (the tunnel-client release bundles it; it is declared as an external binary alongside it in `src-tauri/tauri.conf.json`).

The archive's `LICENSE`, `NOTICE`, SPDX, and license-text files are tracked in git under `src-tauri/binaries/` for attribution.

## First-run setup

1. Launch the app.
2. Click **Add workspace** and pick a folder. Read access is on by default; write is off until you enable it.
3. Create a tunnel endpoint at [platform.openai.com](https://platform.openai.com) (**Settings → Organization → Tunnels**) and note its id and API key.
4. Open **Tunnel settings…**, enter the tunnel id and API key, and save. The status bar shows the sidecar state; when running it reports "connected" (spawn-success — see limitations below).

## Connect ChatGPT

1. In ChatGPT Web, enable **Developer Mode** (Settings → Apps & Connectors / Developer Mode).
2. Create an app/connection with **connection type: Tunnel** and select the tunnel endpoint you created above.
3. Enable the app in a chat and ask it to list/read/edit files in your workspace folder.

## Security model

- **Loopback-only binding**: the MCP server listens strictly on `127.0.0.1:<ephemeral port>` — never on all interfaces. The only external connection is the sidecar's outbound HTTPS to OpenAI.
- **Fail-closed policy pipeline**: every tool invocation passes through one pipeline — pause kill-switch check first, workspace lookup, capability gates (`READ_DISABLED`/`WRITE_DISABLED`), path confinement (canonicalized, component-wise containment; symlinks/junctions cannot escape), sensitive-file deny-list, then execution. Any failure short-circuits to a structured error plus an audit row.
- **Pause kill switch**: one button suspends all MCP tool calls (`ACCESS_PAUSED`); UI operations keep working.
- **Sensitive-file deny-list**: `.env*`, `*.pem`, `*.key`, `id_rsa*`, `.git/config`, `credentials*`, `*.pfx`, `*.p12`, `secrets.*`, and all of `node_modules/**` are blocked at any depth, for reads and writes alike. Overriding requires a code change.
- **Patch-first editing**: edits are anchored string replacements (`apply_patch`), not whole-file writes; every edit carries a `base_revision` (SHA-256 of content as last seen) and fails with `FILE_CHANGED_SINCE_READ` if the file changed externally. New files are written atomically via temp-file + rename.
- **Append-only audit log**: every MCP request writes exactly one audit row (allow, deny, or error) before its response is sent; rows can never be updated or deleted. The latest 100 events are visible in the UI.
- **Secret handling**: the tunnel API key is never written to command-line arguments and never sent back to the UI; it is passed to the sidecar as an environment-variable reference only, and persisted only in the local app-data `settings.json`.

### Known limitations (honesty notes)

- "Connected" means *sidecar process spawned successfully* — control-plane health polling is future work.
- If the app hard-crashes, the sidecar process may be orphaned (no OS-level Job Object binding yet); normal exit tears it down cleanly.

## Error codes

Tool errors surface as structured results `{ code, message, detail? }` with these stable codes:

| Code | Meaning |
|---|---|
| `ACCESS_PAUSED` | kill switch engaged |
| `WORKSPACE_NOT_FOUND` | unknown workspace_id |
| `READ_DISABLED` / `WRITE_DISABLED` | capability off |
| `PATH_OUTSIDE_WORKSPACE` | confinement violation (audited with detail) |
| `SENSITIVE_FILE_BLOCKED` | deny-list hit |
| `FILE_NOT_FOUND` | target missing |
| `FILE_EXISTS` | create collision |
| `INVALID_PATCH` | anchor missing/ambiguous |
| `FILE_CHANGED_SINCE_READ` | revision mismatch |
| `RESULT_TOO_LARGE` | bounds exceeded without truncation possible |
| `INVALID_ARGUMENT` | malformed client input (bad regex/glob, >8 read_files paths) |
| `IO_ERROR` | filesystem failure (OS message included) |

## MCP tool surface

Seven tools, all taking `workspace_id`: `list_directory`, `find_files`, `search_text`, `read_file`, `read_files` (≤8 per call), `create_file` (≤10 MB), `apply_patch`. Read tools return bounded results (e.g. `read_file` ≤ 256 KB window with truncation flag) and carry a `revision` field usable with `apply_patch`.
