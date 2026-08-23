# Local Workspace MCP Manager — Design Spec

Date: 2026-08-23
Status: Approved design (pending user review)
Source plan: [`docs/local-workspace-mcp-manager-codex-master-plan.md`](../../local-workspace-mcp-manager-codex-master-plan.md)

## 1. Summary

A Windows-first desktop app that lets **ChatGPT Web** securely read, search, list, and edit files inside explicitly approved local folders. Built as a single-process Rust/Tauri application embedding an MCP server (Streamable HTTP on `127.0.0.1`) bridged to ChatGPT via OpenAI's official **Secure MCP Tunnel** (`tunnel-client` sidecar).

### Verified assumptions

| Assumption | Verdict |
|---|---|
| Secure MCP Tunnel (openai/tunnel-client v0.0.12) bridges ChatGPT Web ↔ localhost MCP | CONFIRMED |
| User's ChatGPT Plus account has Developer Mode | CONFIRMED (user verified live) |
| rmcp (Rust SDK) supports Streamable HTTP server + tool annotations + structured output | CONFIRMED |
| Single-process Rust (no TS sidecar) viable | CONFIRMED |
| Tauri 2 embeds axum server + managed child processes | CONFIRMED |

### Locked decisions

- Milestone scope: **read / edit / search / list** only. Git integration and process runner are deferred to later phases (policy engine designed for them but no tools shipped).
- Multiple workspaces supported from day one.
- Single-process Rust: Tauri 2 + rmcp + axum bound to `127.0.0.1` + managed `tunnel-client` child process.
- Auth: **`noauth` securityScheme for MVP**. Protection comes from (1) tunnel-client forwarding only authenticated OpenAI traffic, (2) loopback-only binding, (3) policy engine. OAuth 2.1 is a documented post-MVP upgrade phase.
- English UI and documentation.
- Minimal single-screen Tauri UI.

## 2. Architecture

```
┌────────────────────────── Windows ──────────────────────────────┐
│                                                                  │
│  ┌─────────────────── Tauri App (single process) ─────────────┐ │
│  │                                                             │ │
│  │  ┌──────────┐    ┌──────────────────────────────────────┐  │ │
│  │  │ React UI │◄──►│ Rust core                            │  │ │
│  │  │ (1 screen│    │  ├─ Workspace manager (SQLite)       │  │ │
│  │  └──────────┘    │  ├─ Policy engine (fail-closed)      │  │ │
│  │                  │  ├─ Audit log                        │  │ │
│  │                  │  ├─ Kill switch (Pause)              │  │ │
│  │                  │  └─ rmcp MCP server                  │  │ │
│  │                  │      mounted: axum @ 127.0.0.1:PORT  │  │ │
│  │                  │          /mcp (Streamable HTTP)      │  │ │
│  │                  └──────────────────────────────────────┘  │ │
│  │                                                             │ │
│  │  ┌─────────────────────────────────────────────────────┐   │ │
│  │  │ tunnel-client (Go binary, managed child process)    │   │ │
│  │  │ outbound-only HTTPS → api.openai.com                │   │ │
│  │  │ forwards JSON-RPC → http://127.0.0.1:PORT/mcp       │   │ │
│  │  └─────────────────────────────────────────────────────┘   │ │
│  └────────────────────────────────────────────────────────────┘ │
│                              ▲ outbound only                     │
└──────────────────────────────┼───────────────────────────────────┘
                               │
                    ChatGPT Web (Developer Mode app,
                    connection type: Tunnel)
```

Components:

1. **React UI** — control surface. Never touches the filesystem directly; every operation goes through the same Rust core API the MCP tools use.
2. **Rust core** — workspace manager, policy engine, audit log, kill switch, SQLite persistence.
3. **MCP server (rmcp + axum)** — Streamable HTTP at `/mcp`, bound strictly to `127.0.0.1` (avoids Windows Firewall prompt; never `0.0.0.0`).
4. **tunnel-client sidecar** — official OpenAI Go binary bundled via Tauri `bundle.externalBin`, spawned as child process, killed explicitly on app exit.

**Data flow for one MCP request:**
ChatGPT calls tool → tunnel-client forwards to localhost → rmcp dispatches to handler → handler runs the fail-closed pipeline (§4) → structured JSON result back along the same path.

**Isolation principle:** there is exactly ONE doorway to the filesystem — the policy engine. Both MCP tools and UI actions enter through it.

## 3. Data model

SQLite database `manager.db` in the Tauri app-data directory, accessed directly from Rust via `rusqlite` (bundled feature).

### 3.1 `workspaces`

```sql
CREATE TABLE workspaces (
  id         TEXT PRIMARY KEY,        -- uuid v4
  name       TEXT NOT NULL,           -- display name, user-chosen
  root_path  TEXT NOT NULL UNIQUE,    -- canonical path (see §4.2)
  can_read   INTEGER NOT NULL DEFAULT 1,
  can_write  INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,           -- ISO 8601 UTC
  updated_at TEXT NOT NULL
);
```

- `root_path` is canonicalized (symlinks resolved, case normalized) at registration time. All later comparisons use the canonical form.
- UI read/write toggles map directly to `can_read` / `can_write`.

### 3.2 Capability profile (hard-coded constants, no table)

MVP ships a fixed profile embedded in code:

- **Sensitive-file deny list** (matched at any depth): `.env*`, `*.pem`, `*.key`, `id_rsa*`, `.git/config`, `credentials*`, `*.pfx`, `*.p12`, `secrets.*`, plus `node_modules/**` blocked entirely. Read ops return `SENSITIVE_FILE_BLOCKED`; write ops refuse to touch them equally.
- **Result bounds**: `read_file` ≤ 256 KB per call (larger files truncated with `truncated: true`); `read_files` ≤ 8 files per call; `search_text` ≤ 200 matches; `list_directory` ≤ 5,000 entries; `find_files` ≤ 500 paths.
- **Write limits**: max new-file size 10 MB; edits are patch-first (§4.5).

### 3.3 `session_grants`

```sql
CREATE TABLE session_grants (
  id           TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  granted_at   TEXT NOT NULL,
  expires_at   TEXT,                   -- NULL = valid until revoked/app exit
  reason       TEXT NOT NULL           -- "user added via UI" | ...
);
```

Every UI-added workspace gets a permanent grant (`expires_at NULL`). The table exists so the future `request_temporary_permission` tool slots in without schema changes.

### 3.4 `audit_events` (append-only)

```sql
CREATE TABLE audit_events (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  ts          TEXT NOT NULL,           -- ISO 8601 UTC
  source      TEXT NOT NULL,           -- "mcp" | "ui"
  tool        TEXT,                    -- tool name; NULL for pure UI actions
  workspace_id TEXT,                   -- NULL if blocked before resolution
  op          TEXT NOT NULL,           -- "read_file" | "apply_patch" | "pause" | ...
  decision    TEXT NOT NULL,           -- "allow" | "deny" | "error"
  error_code  TEXT,                    -- see §6
  path        TEXT,                    -- canonical path involved, if any
  detail      TEXT                     -- JSON blob (bytes, matches, ...)
);
```

- Append-only: no UPDATE/DELETE anywhere in the codebase. Deleting a workspace does not delete its audit trail.
- Every MCP request writes an audit row before its response is sent — allow, deny, and error alike.
- UI shows the latest 100 rows (time, op, decision, path).

Relations: `Workspace 1─N session_grants`, `Workspace 1─N audit_events`. No user/account tables — single-user desktop app.

## 4. Security & policy engine

### 4.1 Fail-closed request pipeline

Every tool invocation runs this exact sequence; any failure short-circuits to a structured error and an audit row with `decision = "deny"` or `"error"`:

```
1. Kill-switch check        → ACCESS_PAUSED if paused
2. Argument schema validation (rmcp-generated types)
3. Resolve workspace by id  → WORKSPACE_NOT_FOUND
4. Capability check         → READ_DISABLED / WRITE_DISABLED
5. Path confinement         → PATH_OUTSIDE_WORKSPACE / SENSITIVE_FILE_BLOCKED
6. Op-specific validation   → FILE_NOT_FOUND / INVALID_PATCH / FILE_CHANGED_SINCE_READ / ...
7. Execute
8. Audit row (allow)
9. Bounded result (structured JSON)
```

The kill switch is an `AtomicBool` consulted first on every request. While paused, ALL MCP tools return `ACCESS_PAUSED`; UI operations (adding/removing workspaces, viewing audit) keep working. Pause/resume events are audited.

### 4.2 Path confinement (the critical invariant)

Goal: no tool call may observe or modify anything outside the approved workspace root, including through symlinks, junctions, reparse points, `..` segments, or UNC tricks.

Implementation:

1. At registration, compute `root_canonical = dunce::canonicalize(root)` and store it. (`dunce` avoids the `\\?\C:\...` UNC prefix that plain `std::fs::canonicalize` produces on Windows, which breaks string comparisons and naive prefix checks.)
2. At request time, for an existing path: `target_canonical = dunce::canonicalize(joined_path)`, then verify containment by comparing **path components**, not string prefixes: `target_canonical.components()` must start with exactly `root_canonical.components()`.
3. For a path that does not exist yet (`create_file`, new patch targets): canonicalize the nearest existing ancestor, append the remaining non-existent components, and run the same component-wise containment check. This catches a symlink planted inside the tree pointing outward.
4. Absolute paths from the MCP client are rejected outright unless, after joining with the workspace root and canonicalizing, they land inside it — the containment check above is the sole authority; there is no string-prefix fast path.
5. Junction/reparse awareness falls out of step 2–3: `canonicalize` traverses junctions, so an escape through one produces a canonical path outside the root and is refused.

Unit tests MUST cover: `..\` traversal, absolute-path injection, symlink-out, junction-out, case-variation (`WORKSPACE` vs `workspace`), UNC path input, nonexistent nested path under a symlinked ancestor, and the `\\?\` prefix trap itself.

### 4.3 Sensitive-file protection

Deny-list matching runs after confinement, on the path **relative to the workspace root**, using glob patterns (`globset`). Applies identically to reads and writes. The list lives in one module as named constants with tests pinning exact behavior. Overriding requires a code change, not a toggle — deliberate for MVP.

### 4.4 Atomic writes & optimistic concurrency

- Writes go to `<target>.<uuid>.tmp` in the same directory, flushed, then renamed over the target (`std::fs::rename` uses `MOVEFILE_REPLACE_EXISTING` on Windows). Crash mid-write leaves the original intact.
- Read tools return a `revision` field = SHA-256 hex of file content.
- `apply_patch` requires `base_revision`; mismatch → `FILE_CHANGED_SINCE_READ` (optimistic concurrency; prevents blind overwrite of externally-modified files).

### 4.5 Patch-first editing

`apply_patch` arguments: `{ workspace_id, path, old_string, new_string, base_revision }`.

- Semantics mirror the Edit-tool model: `old_string` must occur exactly once in the current file, else `INVALID_PATCH` (ambiguous or absent anchor).
- Chosen over unified-diff parsing because LLMs produce fewer malformed patches this way and error messages stay precise.
- `new_string` empty = deletion of the anchored span; whole-file replacement goes through `old_string` covering the entire prior content (enforced naturally by uniqueness).

### 4.6 Network posture

- axum binds `127.0.0.1:<port>` where port is an ephemeral free port chosen at startup and recorded in app state (shown in UI).
- No inbound listener exposure; the only external connection is tunnel-client's outbound HTTPS to `api.openai.com`.

## 5. MCP tool surface (7 tools)

All tools take `workspace_id` as first argument. All return structured JSON (`structuredContent`) plus human-readable text. Annotations: read tools carry `readOnlyHint: true`; `create_file`/`apply_patch` carry `readOnlyHint: false`.

| Tool | Args | Returns |
|---|---|---|
| `list_directory` | workspace_id, path (default "."), recursive? (bool, default false) | entries[{name, type, size, modified}] |
| `find_files` | workspace_id, pattern (glob) | paths[], truncated flag |
| `search_text` | workspace_id, query (regex or literal), path_prefix? , case_sensitive?, max_results? | matches[{path, line_no, line, col}], truncated flag |
| `read_file` | workspace_id, path, offset_line?, max_lines? | content, revision (sha256), total_lines, truncated flag |
| `read_files` | workspace_id, paths[] (≤8) | per-file result: content or error |
| `create_file` | workspace_id, path, content | created path, revision. Errors `FILE_EXISTS` if target present. |
| `apply_patch` | workspace_id, path, old_string, new_string, base_revision | new revision, bytes changed |

Deliberately NOT in MVP: delete/move/rename tools, git tools, `run_allowed_command`, `request_add_workspace`, `request_temporary_permission`. The pipeline and schema make these additive later.

Search walks with the `ignore` crate (respects `.gitignore`) + `globset` filtering; both crates come from the ripgrep family and skip `node_modules`-style dirs by default, complementing the sensitive-dir block.

## 6. Error model

Structured error codes surfaced as `isError: true` tool results with `{ code, message, detail? }`:

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
| `IO_ERROR` | filesystem failure (OS message included) |

Codes are stable strings — ChatGPT can react to them conversationally ("ask the user to enable write access").

## 7. UI (single screen)

Sections top-to-bottom:

1. **Connection status**: tunnel state (stopped/connecting/connected), local port, tunnel id suffix.
2. **Workspaces list**: rows with name, canonical path, read toggle, write toggle, remove button (remove = confirm dialog; does not delete files; audit preserved).
3. **Add workspace**: native folder-picker dialog (tauri-plugin-dialog), name input defaults to folder name, read ON / write OFF defaults.
4. **Pause button**: large, red when active. Global kill switch.
5. **Audit table**: latest 100 events, newest first.
6. **Settings** (small dialog): tunnel id + OpenAI API key for tunnel-client (§8).

State flows Rust→UI via Tauri events (tunnel status changes push updates; no polling loops).

## 8. Process lifecycle

- App startup (`setup` hook): open DB, run migrations, start axum server on `tauri::async_runtime`, spawn tunnel-client if configured.
- tunnel-client config (tunnel id + API key) stored in a local settings file in app-data, entered once in a small settings dialog. Key never leaves the machine except to api.openai.com.
- Exit (`RunEvent::ExitRequested`): stop accepting requests → kill tunnel-client child → abort axum task → close DB. There is NO automatic child cleanup in Tauri; this explicit teardown is mandatory and tested (orphaned tunnel-client = stale outbound connection).
- Crash safety: SQLite WAL mode; partial writes impossible by construction (§4.4).

## 9. Testing strategy

- **Policy engine unit tests (highest priority)**: full §4.2 matrix, sensitive-file globs, capability gating, pause behavior, revision conflicts, patch anchoring edge cases (multi-occurrence anchor, anchor-at-EOF, CRLF files).
- **Tool-level integration tests**: in-process rmcp client → server over the real axum stack against temp directories; assert both structured results and audit rows.
- **Persistence tests**: migration idempotency, append-only audit enforcement (attempted UPDATE/DELETE rejected in code review + trigger in SQL).
- **Lifecycle tests**: spawn/kill tunnel-client with a stub binary; assert no orphans.
- **E2E (manual, phase-gated)**: real ChatGPT Web conversation performing Scenario A (§10).
- Coverage target ≥80% on the core crate (policy/persistence/tools); UI covered by build + manual smoke.

## 10. Acceptance scenarios

- **A (milestone E2E)**: Add workspace via UI → ChatGPT lists directory → reads a file → applies a patch → verifies change on disk. All four verbs proven over the real tunnel.
- **B (confinement)**: Prompt ChatGPT to read `../../../outside.txt` → `PATH_OUTSIDE_WORKSPACE`, audit row written, nothing leaked.
- **C (sensitive)**: Read `.env` → `SENSITIVE_FILE_BLOCKED`.
- **D (pause)**: Engage Pause mid-conversation → next tool call returns `ACCESS_PAUSED`; resume restores.
- **E (concurrency)**: Modify a file externally after ChatGPT read it → `apply_patch` with stale revision → `FILE_CHANGED_SINCE_READ`; fresh read succeeds.
- **F (write toggle)**: Write access off → `apply_patch` → `WRITE_DISABLED`.
- **G (restart)**: Quit app fully → no orphan tunnel-client process → relaunch reconnects cleanly.
- **H (multi-workspace)**: Two workspaces added; cross-workspace access attempts fail confinement.

## 11. Implementation phases

- **P0 — Skeleton**: Tauri 2 scaffold, axum serving `/mcp` with one echo tool, loopback bind verified. *Verify: curl initialize handshake.*
- **P1 — Core domain**: SQLite + migrations, workspace CRUD, policy engine skeleton, audit log. *Verify: unit tests green.*
- **P2 — Read tools**: list/find/search/read/read_files behind full pipeline. *Verify: integration tests + Scenario H partially.*
- **P3 — Write tools**: create_file, apply_patch, revisions, atomic write. *Verify: Scenarios E, F.*
- **P4 — Tunnel + UI**: bundle tunnel-client, settings dialog, full single-screen UI, lifecycle teardown. *Verify: Scenarios G + A over real ChatGPT Web.*
- **P5 — Hardening**: audit viewer polish, edge-case sweep, packaging (NSIS installer).

Post-MVP backlog (documented, not built): OAuth 2.1 auth, temporary permissions tool, delete/move tools, git tools, process runner, Plugins/Apps SDK custom UI.

## 12. Key dependencies

`tauri 2`, `rmcp` (features: server, transport-streamable-http-server), `axum`, `tokio`, `rusqlite` (bundled), `serde`/`serde_json`, `dunce`, `sha2`, `regex`, `globset`, `ignore`, `uuid` (v4), `time`, `tauri-plugin-dialog`. Frontend: React 18 + TypeScript, Vite.

`tunnel-client` binary obtained from github.com/openai/tunnel-client releases (v0.0.12), placed as sidecar with target-triple naming per Tauri `externalBin` convention.
