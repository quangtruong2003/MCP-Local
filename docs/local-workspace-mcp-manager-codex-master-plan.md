# Local Workspace MCP Manager — Codex Master Implementation Plan

> **For Codex / agentic workers:** Use this document as the master execution brief. Start by inspecting the repository and current official OpenAI/MCP documentation, then implement incrementally. Use Superpowers where useful, but optimize for forward progress: test security-critical/core behavior thoroughly, avoid excessive tests for trivial UI/layout or framework boilerplate, and keep commits/reviews at meaningful boundaries rather than every tiny edit.

**Goal:** Build a Windows-first Local Workspace MCP Manager that lets ChatGPT Web securely read, search, edit, create, and validate files inside explicitly approved local folders through Full MCP + Secure MCP Tunnel, while the user controls workspaces, permissions, tools, sessions, and audit history from a local desktop UI and a smaller optional ChatGPT Apps SDK UI.

**Primary user experience:** The user runs one local manager app, adds folders such as `D:\Meeting Support`, enables only desired capabilities, connects the MCP server through OpenAI Secure MCP Tunnel, then selects the custom app in ChatGPT Web. ChatGPT can operate only within the allowed scope. Changes appear immediately in the real local files.

**Target platform:** Windows first. Keep core abstractions portable enough for macOS/Linux later, but do not delay the Windows release for cross-platform polish.

**Status of OpenAI features:** Full MCP and write/modify actions are beta and may change. Do not assume undocumented commands, flags, endpoints, approval semantics, or Secure MCP Tunnel APIs. Verify the latest official OpenAI documentation before implementing integration-specific code.

---

## 0. Agent operating instructions

### 0.1 Start with repository discovery

Before modifying anything:

1. Inspect the repository tree.
2. Read `README*`, `AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING*`, existing architecture docs, package manifests, Rust manifests, and current tests.
3. Inspect recent commits if Git history is available.
4. Determine whether this is:
   - a new standalone application,
   - an existing Tauri/Rust app,
   - an MCP server already in progress,
   - or a monorepo that should receive this as a new package.
5. Follow existing repository conventions where sensible.
6. Do not perform unrelated refactors.

If the repository is empty/new, use the reference structure in this document.

### 0.2 Verify external contracts before implementation

Use current official sources, prioritizing:

- OpenAI Help / Developer docs for:
  - Developer Mode
  - Full MCP custom apps
  - Secure MCP Tunnel
  - Apps SDK
  - write/modify action confirmation behavior
- Model Context Protocol official specification and SDK documentation.
- Tauri official documentation.

Important facts to re-confirm at execution time:

- ChatGPT Web cannot connect directly to a localhost MCP server.
- Secure MCP Tunnel is the intended path for supported OpenAI products to reach private/on-prem/developer-machine MCP servers without public exposure.
- Full MCP write/modify support and plan availability.
- Current MCP transport expected by ChatGPT.
- Current Apps SDK component/resource conventions.
- Current authentication requirements and supported auth modes.

If the docs differ from assumptions in this plan, follow the official docs and document the adjustment in `docs/architecture-decisions.md`.

### 0.3 Autonomy rules

Proceed without repeatedly asking for approval when the work is:

- reading repository files,
- modifying in-scope local project code,
- adding required dependencies,
- running non-destructive build/test/lint/format commands,
- creating internal docs,
- repairing issues caused by the implementation.

Stop and ask only for a genuine blocker, such as:

- an external credential/token that cannot be obtained automatically,
- a destructive migration affecting user data,
- a major product decision with two materially different user-facing outcomes,
- an OpenAI beta limitation that makes the requested architecture impossible as specified.

Do not ask questions merely because a minor implementation detail is unspecified. Make a reasonable, documented choice.

### 0.4 Testing philosophy

Prioritize tests for:

- path sandbox escape prevention,
- symlink/junction/reparse-point handling,
- permission enforcement,
- session grants,
- command allowlisting and argument safety,
- atomic file writes,
- optimistic concurrency,
- sensitive-file denial,
- audit event correctness,
- MCP tool schemas and error contracts.

Use normal integration tests for:

- workspace add/remove/switch,
- file read/search/edit lifecycle,
- Git tools,
- process execution,
- configuration persistence,
- tunnel-state integration where it can be tested locally.

Do not create large volumes of low-value snapshot/UI tests. Verify UI behavior with focused component tests only where state logic is non-trivial.

---

# 1. Product definition

The product has two complementary control surfaces.

## 1.1 Local desktop manager — source of authority

The local app is the authoritative security/control plane.

It must provide:

- MCP server status.
- Secure MCP Tunnel status.
- Start/stop/pause access.
- Workspace management.
- Native folder picker.
- Per-workspace permissions.
- Global capability/tool toggles.
- Command allowlist.
- Temporary session permissions.
- Activity/audit log.
- Diff/history view for changes.
- Connection diagnostics.
- Emergency kill switch: **Pause ChatGPT Access**.

The local app remains authoritative even if a ChatGPT embedded UI exists.

## 1.2 ChatGPT Apps SDK UI — convenience surface

The embedded ChatGPT UI should expose a safe subset:

- connection status,
- active workspace,
- workspace switching among already-approved workspaces,
- high-level capability status,
- request temporary privilege,
- recent activity,
- open/request local manager interaction.

It must **not** silently grant itself new filesystem roots or high-risk permissions.

If ChatGPT requests adding a new folder:

1. MCP records a local-interaction request.
2. Local Manager displays/opens a native folder picker.
3. The human selects the folder locally.
4. Local policy engine stores the approved workspace.
5. MCP returns the result.

If local interactive approval cannot be performed, return a structured `LOCAL_INTERACTION_REQUIRED` result rather than guessing.

---

# 2. High-level architecture

```text
┌─────────────────────────────────────────────────────────────┐
│                         ChatGPT Web                         │
│                                                             │
│ Custom MCP App + optional Apps SDK interactive component    │
└──────────────────────────────┬──────────────────────────────┘
                               │
                         Full MCP calls
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                    Secure MCP Tunnel                        │
│         Private connection; no public MCP exposure          │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│               Local Workspace MCP Manager                   │
│                                                             │
│  MCP Adapter / Server                                       │
│          │                                                  │
│          ▼                                                  │
│  Capability / Policy Engine ───── Session Grant Manager     │
│          │                         │                         │
│          ├──────────────┬──────────┴─────────────┐           │
│          ▼              ▼                        ▼           │
│    Workspace        Audit Service          Tunnel Service    │
│      Service                                                │
│          │                                                  │
│    ┌─────┼─────────────┬──────────────┐                     │
│    ▼     ▼             ▼              ▼                     │
│ Files  Search          Git        Process Runner            │
│                                                             │
│                     Local Tauri UI                          │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
                     Approved local folders
```

### Architectural invariant

Every filesystem or process request must follow:

```text
MCP request
  -> schema validation
  -> resolve workspace
  -> policy evaluation
  -> path canonicalization / confinement check
  -> operation-specific validation
  -> execute
  -> audit
  -> return bounded/redacted result
```

No adapter is allowed to bypass the policy engine.

---

# 3. Recommended technology strategy

## 3.1 Desktop/control plane

Preferred:

- **Tauri 2**
- **Rust**
- **React + TypeScript**
- modern lightweight component system consistent with the existing repo
- SQLite for persistent settings/audit/session state

Recommended Rust concerns:

- `tokio` for async work.
- `serde` / `serde_json` for contracts.
- SQLite library consistent with repository conventions (`rusqlite` is acceptable for a local app; `sqlx` is also acceptable if already present).
- `tracing` for structured diagnostics.
- `thiserror` / equivalent typed errors.
- a mature Git integration approach:
  - prefer invoking `git` with argument arrays for exact CLI compatibility,
  - or use `git2` only if the repository already relies on libgit2 and its behavior is sufficient.

## 3.2 MCP implementation decision gate

Do not choose an MCP SDK from memory.

During Phase 0, evaluate current official/stable SDK support for:

- required ChatGPT transport,
- Full MCP tools,
- write actions,
- tool annotations/metadata,
- Apps SDK resource/component integration,
- streaming where applicable,
- authentication,
- Windows packaging.

Preferred outcome:

### Option A — single-process Rust

Use a stable Rust MCP implementation if it fully supports the required current MCP/OpenAI contract.

Benefits:

- one executable,
- easier local privilege boundary,
- simpler packaging,
- shared Rust policy engine directly.

### Option B — Rust manager + TypeScript MCP sidecar

Use this fallback if the official/current TypeScript MCP/Apps SDK stack is materially more complete than the Rust ecosystem for the required ChatGPT features.

Architecture:

```text
Tauri/Rust local authority
        │
 authenticated loopback IPC
        │
 TypeScript MCP sidecar
        │
 Secure MCP Tunnel
```

Rules for Option B:

- The TypeScript sidecar must not receive raw unrestricted filesystem authority.
- File/process operations remain in Rust.
- Sidecar calls the Rust authority through a private authenticated local IPC/API.
- Bind loopback only.
- Generate a random per-install/per-launch local secret.
- Keep the IPC surface narrow and typed.

Record the choice and evidence in:

`docs/architecture-decisions.md`

---

# 4. Reference repository layout

Adapt to existing repo conventions where necessary.

```text
/
├─ src/                              # React/Tauri frontend if standard Tauri layout
│  ├─ app/
│  ├─ components/
│  ├─ features/
│  │  ├─ overview/
│  │  ├─ workspaces/
│  │  ├─ permissions/
│  │  ├─ tools/
│  │  ├─ activity/
│  │  ├─ tunnel/
│  │  └─ settings/
│  └─ lib/
│
├─ src-tauri/
│  ├─ src/
│  │  ├─ main.rs
│  │  ├─ app_state.rs
│  │  ├─ db/
│  │  │  ├─ mod.rs
│  │  │  ├─ migrations.rs
│  │  │  └─ repositories/
│  │  ├─ domain/
│  │  │  ├─ workspace.rs
│  │  │  ├─ capability.rs
│  │  │  ├─ permission.rs
│  │  │  ├─ session.rs
│  │  │  ├─ audit.rs
│  │  │  └─ errors.rs
│  │  ├─ policy/
│  │  │  ├─ mod.rs
│  │  │  ├─ evaluator.rs
│  │  │  ├─ path_guard.rs
│  │  │  ├─ sensitive_files.rs
│  │  │  └─ command_policy.rs
│  │  ├─ services/
│  │  │  ├─ workspace_service.rs
│  │  │  ├─ filesystem_service.rs
│  │  │  ├─ search_service.rs
│  │  │  ├─ git_service.rs
│  │  │  ├─ process_service.rs
│  │  │  ├─ audit_service.rs
│  │  │  ├─ session_service.rs
│  │  │  └─ tunnel_service.rs
│  │  ├─ mcp/
│  │  │  ├─ mod.rs
│  │  │  ├─ server.rs
│  │  │  ├─ tools/
│  │  │  ├─ schemas/
│  │  │  └─ errors.rs
│  │  └─ tauri_commands/
│  │     ├─ workspaces.rs
│  │     ├─ permissions.rs
│  │     ├─ activity.rs
│  │     ├─ tunnel.rs
│  │     └─ settings.rs
│  └─ tests/
│     ├─ policy/
│     ├─ filesystem/
│     ├─ mcp/
│     └─ integration/
│
├─ mcp-sidecar/                      # Only if Option B is selected
│  ├─ src/
│  ├─ package.json
│  └─ tests/
│
├─ chatgpt-app/                      # Apps SDK UI assets/resources if separate
│  ├─ src/
│  ├─ package.json
│  └─ README.md
│
├─ docs/
│  ├─ architecture.md
│  ├─ architecture-decisions.md
│  ├─ security-model.md
│  ├─ chatgpt-setup.md
│  └─ troubleshooting.md
│
└─ README.md
```

Do not create empty layers merely to match this tree. Each module should exist only when it has a clear responsibility.

---

# 5. Core domain model

Use explicit stable IDs rather than filesystem paths as identifiers.

## 5.1 Workspace

Conceptual contract:

```rust
struct Workspace {
    id: WorkspaceId,
    name: String,
    root_path: PathBuf,
    enabled: bool,
    created_at: DateTime,
    updated_at: DateTime,
}
```

Important behavior:

- `root_path` is stored locally only.
- ChatGPT should normally see `workspace_id`, display name, and relative paths.
- Do not expose unrelated absolute local paths in normal MCP responses.
- Each request must resolve relative paths against the selected workspace.

## 5.2 Capability profile

Support presets:

- `read_only`
- `developer`
- `full_access`
- `custom`

But persist actual granular permissions so presets are only UI conveniences.

Minimum permissions:

```text
files.read
files.search
files.create
files.modify
files.move
files.delete

git.status
git.diff
git.log
git.commit
git.checkout

process.build
process.test
process.lint
process.allowed_command
process.arbitrary_shell

workspace.switch
workspace.request_add
workspace.remove
```

Default profile for a newly added coding workspace:

```text
files.read              ON
files.search            ON
files.create            ON
files.modify            ON
files.move              OFF
files.delete            OFF

git.status              ON
git.diff                ON
git.log                 ON
git.commit              OFF
git.checkout            OFF

process.build           ON
process.test            ON
process.lint            ON
process.allowed_command ON
process.arbitrary_shell OFF
```

## 5.3 Session grants

A permission may be:

- denied,
- always allowed,
- allowed once,
- allowed until a local expiration time,
- allowed for the current ChatGPT/MCP session when a reliable session identifier is available.

Do not invent a ChatGPT session identity if MCP does not provide one reliably. If no stable session identity exists, model temporary access as a local grant with an explicit expiry/remaining-use count.

Example:

```rust
enum GrantScope {
    Once,
    Until(InstantOrTimestamp),
    Persistent,
}
```

## 5.4 Audit event

Capture:

- event ID,
- timestamp,
- source (`chatgpt_mcp`, `local_ui`, `system`),
- tool/action,
- workspace ID,
- relative paths,
- permission decision,
- result status,
- duration,
- bounded metadata,
- optional diff summary,
- process exit code,
- output truncation/redaction metadata.

Never store secrets or full environment dumps in audit logs.

---

# 6. Security model — non-negotiable

## 6.1 Workspace confinement

All path-based operations must:

1. Reject absolute paths from MCP unless a specific internal-only endpoint requires one.
2. Normalize separators.
3. reject NUL / malformed path inputs.
4. join relative path to workspace root.
5. resolve canonical/reparse-point-aware destination as safely as Windows permits.
6. verify the final target remains inside the approved workspace.
7. repeat confinement validation where an operation creates or moves a path.

Explicitly test:

- `..\` traversal,
- mixed `/` and `\`,
- symlinks,
- Windows junctions,
- reparse points,
- case-insensitive path behavior,
- UNC paths,
- drive-relative paths such as `C:foo`,
- alternate path forms,
- move/rename from inside to outside,
- symlink replacement between validation and use where relevant.

Avoid naive string prefix checks.

## 6.2 Sensitive-file policy

Default deny or require an explicit local override for known secret-bearing files, including reasonable patterns such as:

- `.env`
- `.env.*` except `.env.example` / documented templates
- private keys
- credential stores
- `.ssh`
- cloud provider credential files
- auth token stores
- browser profile secrets
- OS credential databases

Do not block ordinary source code merely because it contains words like `token` or `secret`.

Sensitive-file policy is additional to workspace confinement.

## 6.3 File writes

Implement:

- atomic replace where possible,
- parent directory validation,
- write-size limits,
- encoding-aware behavior,
- preservation of line endings when practical,
- optimistic concurrency.

For read operations that may later be edited, return a revision token derived from file state/content.

Conceptual result:

```json
{
  "path": "src/audio/diarization.rs",
  "revision": "sha256:...",
  "content": "..."
}
```

Write/patch requests may include:

```json
{
  "expected_revision": "sha256:..."
}
```

If mismatched:

```text
FILE_CHANGED_SINCE_READ
```

The model must re-read before retrying.

## 6.4 Patch-first editing

Prefer `apply_patch` over full-file replacement.

`write_file` remains available for:

- new files,
- small generated files,
- explicit complete replacements.

For existing code, MCP tool descriptions should guide ChatGPT toward `apply_patch`.

## 6.5 Safe delete

Default implementation:

- capability disabled.
- when enabled, move deleted files into an app-managed local trash/recovery area when feasible.
- record source path and recovery path.
- do not permanently erase by default.

Provide an explicit cleanup function in the local UI, not an MCP tool in MVP.

## 6.6 Process execution

Never concatenate a model-produced string into `cmd.exe /c` or PowerShell.

Model commands structurally:

```json
{
  "program": "cargo",
  "args": ["test", "--workspace"],
  "cwd": ".",
  "timeout_seconds": 300
}
```

Allowlist executable names/paths.

Initial allowed coding commands may include user-configurable entries such as:

- `cargo`
- `rustc`
- `npm`
- `pnpm`
- `yarn`
- `node`
- `git`
- project-specific test/build tools

Default deny:

- `cmd.exe`
- `powershell.exe`
- `pwsh.exe`
- `reg.exe`
- arbitrary interpreters unless explicitly approved
- commands outside allowlist

Do not inherit all secrets blindly. Construct a controlled environment or at minimum redact sensitive environment values from returned output/logs.

Apply:

- timeout,
- output byte limit,
- process-tree termination on timeout/cancel,
- cwd confinement,
- safe UTF-8/lossy output handling,
- exit code reporting.

## 6.7 Network posture

- Local authority binds only to loopback/private IPC as required.
- Do not expose the raw MCP server publicly merely to make ChatGPT reach it.
- Use Secure MCP Tunnel according to current official OpenAI instructions.
- If an MCP sidecar is used, its IPC must be authenticated and loopback-only.
- Do not disable TLS/auth protections to “make it work.”

## 6.8 Kill switch

The local UI must have:

**Pause ChatGPT Access**

When paused:

- MCP operations that touch local resources fail closed.
- Existing long-running model-triggered processes should be canceled if safe/appropriate.
- local UI remains usable.
- tunnel may remain connected for diagnostics, but no privileged operation succeeds.

---

# 7. MCP tool surface

Keep the public tool set compact and semantically strong. Target roughly 12–20 tools, not dozens of tiny operations.

Tool names may be adjusted to current MCP/OpenAI conventions, but preserve these responsibilities.

## 7.1 Discovery/status

### `get_workspace_context`

Returns:

- active workspace,
- enabled capabilities,
- project summary if cheaply available,
- repository/Git status summary,
- relevant limits.

### `list_workspaces`

Returns only approved/enabled workspaces and safe metadata.

### `select_workspace`

Switches active workspace if permitted.

### `get_capabilities`

Returns current effective permissions so ChatGPT can avoid futile calls.

## 7.2 Search/read

### `list_directory`

Inputs:

- workspace ID optional,
- relative path,
- depth,
- bounded result count.

### `find_files`

Inputs:

- filename/glob/query,
- relative root,
- result limit.

### `search_text`

Inputs:

- query,
- relative root,
- include/exclude patterns,
- result limit,
- context lines.

Use an efficient local search implementation. If `rg` is available and safely invoked, it is acceptable; otherwise use a Rust implementation.

### `read_file`

Inputs:

- relative path,
- optional line/byte range,
- bounded max output.

Returns content plus revision.

### `read_files`

Allow a small bounded batch to reduce tool-call overhead.

## 7.3 Editing

### `apply_patch`

Requirements:

- relative path,
- explicit patch format supported by the implementation,
- optional expected revision,
- dry validation before write,
- atomic application,
- structured conflict errors.

### `create_file`

Requirements:

- relative path,
- content,
- fail if exists unless explicit safe overwrite mode is allowed,
- permission check.

### `move_file`

Disabled by default in Developer profile.

### `safe_delete`

Disabled by default.

## 7.4 Git

### `git_status`

### `git_diff`

Support scoped diff for a file/path and bounded output.

### `git_log`

Bound result count.

MVP should not expose `git reset --hard`, destructive checkout, force push, or similar actions.

`git_commit` may be added later as an opt-in capability.

## 7.5 Validation/process

### `run_task`

Preferred high-level interface.

Examples:

```text
cargo_check
cargo_test
npm_test
npm_build
pnpm_test
lint
format_check
```

Discover common project tasks from manifests and let the local user enable them.

### `run_allowed_command`

Secondary structured interface:

- program,
- args array,
- relative cwd,
- timeout.

No raw shell string.

## 7.6 Local-interaction requests

### `request_add_workspace`

This does **not** accept a raw arbitrary path from the model.

Behavior:

1. create a local interaction request,
2. notify/open Local Manager if possible,
3. require native human folder selection,
4. return approved workspace or `LOCAL_INTERACTION_REQUIRED` / canceled status.

### `request_temporary_permission`

Inputs:

- requested capability,
- reason,
- desired duration/scope if supported.

The local UI displays the request. The model cannot approve its own request.

---

# 8. Structured MCP error model

Avoid opaque free-form errors.

Use stable error codes such as:

```text
ACCESS_PAUSED
PERMISSION_DENIED
WORKSPACE_DISABLED
WORKSPACE_NOT_FOUND
PATH_OUTSIDE_WORKSPACE
SENSITIVE_FILE_BLOCKED
FILE_NOT_FOUND
FILE_ALREADY_EXISTS
FILE_CHANGED_SINCE_READ
PATCH_REJECTED
RESULT_TRUNCATED
COMMAND_NOT_ALLOWED
COMMAND_TIMEOUT
PROCESS_FAILED
LOCAL_INTERACTION_REQUIRED
LOCAL_INTERACTION_CANCELED
TUNNEL_UNAVAILABLE
INTERNAL_ERROR
```

Return:

- stable code,
- concise safe message,
- whether retry may help,
- actionable next step where appropriate.

Do not leak unrestricted absolute paths or secrets in errors.

---

# 9. Persistence model

Use SQLite unless the existing repository already has a better established persistence layer.

Minimum tables/entities:

```text
workspaces
workspace_permissions
global_settings
command_allowlist
session_grants
audit_events
local_interaction_requests
app_state / migrations
```

Requirements:

- schema versioning/migrations,
- transactions for permission/workspace updates,
- clean startup recovery,
- no secret storage in plaintext unless unavoidable and explicitly documented,
- purge/retention settings for audit data.

Recommended audit retention default:

- bounded by age and/or row count,
- configurable in Local Manager.

---

# 10. Phase-by-phase implementation

## Phase 0 — Discovery and technical spike

**Outcome:** The repository strategy, MCP SDK choice, current Secure MCP Tunnel integration contract, and Apps SDK strategy are proven before large implementation begins.

Tasks:

- [ ] Map current repository and build system.
- [ ] Confirm current OpenAI Full MCP requirements from official docs.
- [ ] Confirm current Secure MCP Tunnel setup and lifecycle from official docs.
- [ ] Confirm current MCP transport/protocol version.
- [ ] Confirm current Apps SDK UI resource/component requirements.
- [ ] Evaluate Rust MCP implementation vs TypeScript sidecar.
- [ ] Build the smallest throwaway/isolated MCP proof that exposes a harmless `ping`/`get_status` tool.
- [ ] Verify it can be discovered by ChatGPT through the supported tunnel path in a development workspace, if credentials/account access are available.
- [ ] Record unresolved external/account blockers explicitly if the actual ChatGPT connection cannot yet be exercised.
- [ ] Write `docs/architecture-decisions.md`.
- [ ] Remove throwaway code or promote only clean reusable pieces.

**Gate:** Do not build the full UI before the MCP transport/tunnel architecture is understood.

---

## Phase 1 — Application foundation

**Outcome:** Local app launches reliably and has persistent application state.

- [ ] Establish Tauri/Rust + React/TS structure if not already present.
- [ ] Add structured logging.
- [ ] Add typed application error handling.
- [ ] Add SQLite initialization and migrations.
- [ ] Define domain types for workspace, permissions, grants, audit events, tunnel state.
- [ ] Add app state/services composition root.
- [ ] Add a minimal Overview screen.
- [ ] Add application-level health status.
- [ ] Run build, format, lint, and focused tests.

Acceptance:

- App starts from a clean profile.
- DB initializes automatically.
- Restart preserves settings.
- A failed migration produces a clear recoverable error rather than silent corruption.

---

## Phase 2 — Workspace Manager

**Outcome:** User can safely add, remove, enable/disable, and switch local folders.

- [ ] Implement native folder picker through Tauri.
- [ ] Add workspace persistence.
- [ ] Add unique stable workspace IDs.
- [ ] Validate folder existence/access before approval.
- [ ] Add active workspace logic.
- [ ] Add enable/disable workspace.
- [ ] Add remove workspace with confirmation in local UI.
- [ ] Build Workspaces screen.
- [ ] Add tests for persistence and invalid paths.

UI behavior:

```text
Workspaces

● Meeting Support
  D:\Meeting Support
  Developer
  [Disable] [Manage]

○ Liberty Odyssea
  D:\projectfl\liberty-odyssea
  Read only
  [Enable] [Manage]

[ + Add Folder ]
```

Do not send absolute root paths to ChatGPT unless needed for a local-only diagnostic.

---

## Phase 3 — Policy engine and path sandbox

**Outcome:** There is one authoritative fail-closed permission/security layer.

- [ ] Implement capability enums and effective-permission resolution.
- [ ] Implement per-workspace permission profiles.
- [ ] Implement global tool/capability toggles.
- [ ] Implement path guard.
- [ ] Implement sensitive-file policy.
- [ ] Implement access-pause global gate.
- [ ] Implement structured authorization decision object.
- [ ] Unit test all known path escape cases.
- [ ] Test Windows junction/reparse behavior on Windows CI/local test where possible.
- [ ] Ensure every resource service consumes an authorization decision rather than rolling its own logic.

**Gate:** No MCP write/process tool may be enabled until this phase is passing.

---

## Phase 4 — Read/search filesystem services

**Outcome:** Approved workspace files can be explored efficiently without allowing escape.

- [ ] Implement bounded directory listing.
- [ ] Implement filename discovery.
- [ ] Implement text search.
- [ ] Implement ranged file read.
- [ ] Implement bounded multi-file read.
- [ ] Add revision/hash generation.
- [ ] Add content/result size limits.
- [ ] Add binary-file detection/handling.
- [ ] Add audit events.
- [ ] Add integration tests against temporary workspace fixtures.

Acceptance:

- Searching a project does not require returning the full tree.
- Large files do not flood MCP context.
- Paths returned to ChatGPT are workspace-relative.
- Reads of blocked secret files fail with `SENSITIVE_FILE_BLOCKED`.

---

## Phase 5 — Safe editing services

**Outcome:** ChatGPT can create and patch real local files safely.

- [ ] Implement atomic new-file creation.
- [ ] Implement patch validation and application.
- [ ] Implement expected revision concurrency checks.
- [ ] Implement safe move/rename behind a disabled-by-default capability.
- [ ] Implement recoverable safe delete behind a disabled-by-default capability.
- [ ] Produce bounded change summaries/diffs for audit.
- [ ] Add tests for concurrent modification.
- [ ] Add tests for failed patch preserving original file.
- [ ] Add tests for move/delete escape attempts.
- [ ] Verify changes are immediately visible to external editors.

Acceptance:

- A stale revision never silently overwrites a newer edit.
- A failed patch leaves the original file intact.
- No write can target outside the workspace.

---

## Phase 6 — Git service

**Outcome:** ChatGPT can inspect its changes without broad shell access.

- [ ] Detect whether active workspace is a Git repository.
- [ ] Implement `git status`.
- [ ] Implement bounded `git diff`.
- [ ] Implement bounded `git log`.
- [ ] Sanitize/limit output.
- [ ] Audit every operation.
- [ ] Handle non-Git workspaces cleanly.
- [ ] Add integration tests with a temporary Git repository.

MVP default:

```text
status   ON
diff     ON
log      ON
commit   OFF
checkout OFF
reset    NOT EXPOSED
```

---

## Phase 7 — Safe process runner

**Outcome:** ChatGPT can run build/test/lint tasks and explicitly allowed executables without arbitrary shell execution.

- [ ] Implement command configuration model.
- [ ] Implement executable allowlist.
- [ ] Implement structured args-only spawning.
- [ ] Implement workspace-confined cwd.
- [ ] Implement timeout.
- [ ] Implement process tree cancellation.
- [ ] Implement stdout/stderr output caps.
- [ ] Implement output redaction.
- [ ] Implement high-level discovered project tasks.
- [ ] Add local UI for enabling/disabling commands.
- [ ] Add process audit history.
- [ ] Test shell-metacharacter arguments to prove no shell interpolation occurs.

Acceptance:

- `cargo test`, `npm test`, etc. can run when enabled.
- `cmd.exe /c ...` fails unless the user explicitly creates such a dangerous allowlist entry.
- timeout/cancel leaves no obvious orphan child process.

---

## Phase 8 — Session grants and local approvals

**Outcome:** High-risk access can be granted temporarily without permanently weakening policy.

- [ ] Implement once/until/persistent grant model.
- [ ] Implement request queue.
- [ ] Implement local approval modal.
- [ ] Implement deny/cancel.
- [ ] Implement expiry/remaining-use enforcement.
- [ ] Add `request_temporary_permission`.
- [ ] Add `request_add_workspace`.
- [ ] Ensure the requester cannot self-approve.
- [ ] Audit requests and outcomes.

Example local UI:

```text
Permission request

ChatGPT requests: files.delete
Workspace: Meeting Support
Reason: remove obsolete generated file

( ) Allow once
( ) Allow for 10 minutes
( ) Always allow

[ Deny ]   [ Allow ]
```

Default selection should favor the least persistent safe option.

---

## Phase 9 — MCP server/tool layer

**Outcome:** The local capabilities are exposed through a compact, typed, policy-enforced MCP surface.

- [ ] Implement current required MCP transport.
- [ ] Define tool schemas.
- [ ] Map every tool to service layer calls.
- [ ] Add effective capability metadata.
- [ ] Add structured errors.
- [ ] Add output bounds.
- [ ] Add tool-level descriptions that encourage search/ranged reads/patches instead of full-project dumps/full-file rewrites.
- [ ] Add MCP contract tests.
- [ ] Verify paused mode denies privileged calls.
- [ ] Verify disabled tools fail closed.
- [ ] Verify read-only workspace cannot be modified.

The MCP adapter must contain minimal business logic. Policy belongs in the local authority/core services.

---

## Phase 10 — Secure MCP Tunnel integration

**Outcome:** ChatGPT can reach the private MCP server without exposing it publicly.

Implement against the **current official OpenAI Secure MCP Tunnel documentation**, not assumptions from this plan.

Required product behavior:

- [ ] Detect whether tunnel prerequisites are installed/configured.
- [ ] Provide setup guidance in Local Manager.
- [ ] Provide connection state:
  - disconnected,
  - connecting,
  - connected,
  - degraded,
  - error.
- [ ] Show useful diagnostics without leaking secrets.
- [ ] Support reconnect.
- [ ] Support stop/disconnect if the official integration allows local lifecycle control.
- [ ] If the official tunnel is designed as a separately managed process/service, integrate via safe process supervision/health checks rather than reverse-engineering it.
- [ ] Do not parse unstable human-readable output when a documented machine-readable interface exists.
- [ ] Document manual fallback steps when automatic setup is not officially supported.

Do not invent a private OpenAI API or fake tunnel protocol.

Acceptance:

- MCP is not publicly exposed.
- loss of tunnel connection is reflected accurately.
- local policy remains enforced regardless of tunnel state.

---

## Phase 11 — Local desktop UX

**Outcome:** The manager is pleasant enough for daily use and makes security state obvious.

### Navigation

```text
Overview
Workspaces
Permissions
Tools & Commands
Activity
Tunnel
Settings
```

### Overview

Show:

- overall connection state,
- MCP server state,
- tunnel state,
- active workspace,
- current permission preset,
- recent actions,
- large **Pause ChatGPT Access** button.

### Workspaces

Support:

- add folder,
- remove,
- enable/disable,
- rename display name,
- set active,
- per-workspace permission profile.

### Permissions

Use grouped controls:

```text
Files
Search
Git
Commands
Workspace management
```

Provide presets plus Custom.

### Activity

Display:

- time,
- action,
- workspace,
- path/task,
- result,
- duration.

Click an edit event to show bounded diff/summary.

### Tunnel

Show:

- status,
- last connected time,
- diagnostics,
- reconnect/stop where supported.

### UX rules

- Dangerous permissions have explicit visual severity.
- Do not make Full Access the default.
- Use concise explanatory copy.
- Avoid modal spam for ordinary safe operations.
- High-risk local approvals remain clear and deliberate.
- Local kill switch is always easy to find.

---

## Phase 12 — ChatGPT Apps SDK UI

**Outcome:** ChatGPT can render a compact manager/status component without replacing local authority.

Verify current Apps SDK requirements first.

Target embedded experience:

```text
Local Workspace MCP
● Connected

Active workspace
[ Meeting Support ▾ ]

Access
Read/Search       On
Modify/Create     On
Delete            Off
Build/Test        On
Arbitrary shell   Off

Recent activity
• modified src/...
• cargo check passed

[Request permission] [Open local manager]
```

Capabilities:

- [ ] render connection/status.
- [ ] list/select pre-approved workspaces.
- [ ] show effective capabilities.
- [ ] request temporary privilege.
- [ ] show recent bounded activity.
- [ ] request local folder-add interaction.
- [ ] never grant a new filesystem root directly from the embedded UI without local human approval.
- [ ] follow current OpenAI UI/resource security guidance.

If Apps SDK write/interactive behavior differs from this design, preserve the authority rule and adapt UI behavior.

---

## Phase 13 — ChatGPT Developer Mode/custom app setup

**Outcome:** A user/admin can connect the built MCP app to ChatGPT Web reproducibly.

Create `docs/chatgpt-setup.md` using current UI terminology.

Cover:

1. prerequisites/eligible plan,
2. enable Developer Mode,
3. run Local Workspace MCP Manager,
4. establish Secure MCP Tunnel,
5. create custom app,
6. provide MCP endpoint/metadata,
7. select authentication if required,
8. scan tools,
9. create/test the draft app,
10. select app in a new ChatGPT chat,
11. test read,
12. test write with a temporary non-critical workspace,
13. verify confirmation behavior,
14. publish only after security review if applicable.

Business vs Enterprise/Edu differences should be documented only after re-checking current official docs.

---

## Phase 14 — Security hardening and abuse testing

**Outcome:** The system resists obvious prompt-induced or path/process escape attempts.

Create an adversarial test matrix including:

### Filesystem

- traversal,
- symlink/junction escape,
- alternate drive path,
- UNC path,
- create-through-link,
- rename outside root,
- delete outside root,
- sensitive file reads,
- hidden/system files,
- oversized files.

### Process

- shell metacharacters inside args,
- attempts to invoke shell,
- path to executable outside allowlist,
- cwd escape,
- environment secret echo,
- huge output,
- infinite process,
- spawned child processes.

### Permissions

- disabled capability call,
- expired grant,
- once grant used twice,
- workspace disabled during operation,
- global pause during operation,
- local denial,
- request spoofing.

### MCP

- malformed schemas,
- oversized tool arguments,
- unexpected enum values,
- repeated retries,
- partial failures,
- tool output truncation.

Fix security failures before release.

---

## Phase 15 — Packaging, startup, and recovery

**Outcome:** The Windows app behaves like a normal desktop utility.

- [ ] Package signed/release build according to project capabilities.
- [ ] Add optional launch-on-login setting.
- [ ] Prefer tray behavior if appropriate.
- [ ] Ensure only one authoritative manager instance owns the DB/server.
- [ ] Add clean shutdown.
- [ ] Recover from stale tunnel/sidecar state.
- [ ] Store logs in an appropriate local app data directory.
- [ ] Add diagnostic export that redacts secrets.
- [ ] Validate uninstall does not unexpectedly delete user project files.
- [ ] Decide/document whether app DB/audit state remains after uninstall.

---

# 11. MVP scope

The first usable release should include:

- Local Tauri manager.
- Add/remove/enable/switch workspace.
- Workspace sandbox.
- Read/search/list files.
- `apply_patch`.
- Create file.
- Git status/diff/log.
- Safe build/test runner.
- Tool/permission toggles.
- Developer/read-only presets.
- Audit log.
- Pause access.
- MCP connection.
- Secure MCP Tunnel integration/setup.
- ChatGPT Developer Mode setup docs.

Can be deferred until after MVP if needed:

- embedded Apps SDK manager UI,
- safe delete recovery browser,
- Git commit,
- elaborate task auto-discovery,
- multi-user policies,
- macOS/Linux packaging,
- advanced analytics,
- theme customization.

Do not delay the secure core/MCP workflow to polish deferred features.

---

# 12. End-to-end acceptance scenarios

## Scenario A — Read-only audit

Configuration:

```text
Workspace: DemoProject
Profile: Read Only
```

Prompt:

> Inspect the repository and explain where authentication is implemented.

Expected:

- ChatGPT lists/searches/reads through MCP.
- No writes occur.
- attempted write is denied.
- all paths shown to model are relative where feasible.
- audit log records reads/searches.

## Scenario B — Safe code edit

Profile:

```text
Developer
```

Prompt:

> Find the hard-coded timeout, make it configurable, then run the relevant tests.

Expected:

1. search,
2. targeted reads,
3. `apply_patch`,
4. test/build task,
5. `git_diff`,
6. local real file is updated,
7. audit log shows actions,
8. no arbitrary shell access was needed.

## Scenario C — Concurrent local edit

1. ChatGPT reads `src/example.rs`.
2. User edits it in VS Code.
3. ChatGPT submits patch with old revision.

Expected:

```text
FILE_CHANGED_SINCE_READ
```

No overwrite.

## Scenario D — Traversal attempt

Tool input:

```text
../../Windows/System32/...
```

Expected:

```text
PATH_OUTSIDE_WORKSPACE
```

No filesystem access.

## Scenario E — Secret file

Within workspace:

```text
.env
```

Expected default:

```text
SENSITIVE_FILE_BLOCKED
```

User may explicitly change local policy if desired.

## Scenario F — Delete request

`files.delete = OFF`

Expected:

- model gets `PERMISSION_DENIED` or uses `request_temporary_permission`,
- local human chooses Allow once/Deny,
- model cannot self-approve.

## Scenario G — Pause

While connected:

1. user presses **Pause ChatGPT Access**,
2. ChatGPT attempts `read_file`.

Expected:

```text
ACCESS_PAUSED
```

## Scenario H — Add workspace from ChatGPT

Prompt:

> Add another project folder.

Expected:

- MCP creates local interaction request.
- native local UI asks the human to choose a folder.
- ChatGPT does not provide/force an arbitrary local root itself.
- selected folder becomes a workspace only after local approval.

---

# 13. Performance requirements

Do not optimize prematurely, but avoid obvious context/performance waste.

Targets:

- UI remains responsive during searches/builds.
- large file operations are bounded.
- search results are capped and paginatable/continuable if protocol UX benefits.
- process output streams/buffers safely and is capped.
- MCP tool schemas are concise.
- `get_workspace_context` does not recursively scan the entire repository on every call.
- cache only metadata that can be invalidated safely.
- do not send complete file trees by default.

---

# 14. Observability

Use structured logging with categories such as:

```text
mcp
policy
workspace
filesystem
search
git
process
tunnel
audit
ui
```

Logs should support:

- level,
- timestamp,
- operation ID/correlation ID,
- workspace ID,
- safe relative path,
- duration,
- result code.

Never log:

- file contents by default,
- auth tokens,
- raw environment,
- private key material.

Add a diagnostics view/export with redaction.

---

# 15. Documentation deliverables

Before declaring completion, produce/update:

### `README.md`

- what the product does,
- security model summary,
- development setup,
- build/run.

### `docs/architecture.md`

- components,
- data flow,
- authority boundaries,
- MCP flow,
- local approval flow.

### `docs/security-model.md`

- threat model,
- path confinement,
- permissions,
- sensitive files,
- process runner,
- tunnel posture,
- known limitations.

### `docs/chatgpt-setup.md`

- current official ChatGPT/Developer Mode/Secure MCP Tunnel steps.

### `docs/troubleshooting.md`

At minimum:

- MCP server not running,
- tunnel disconnected,
- ChatGPT cannot scan tools,
- permission denied,
- workspace unavailable,
- command not allowed,
- stale revision conflict,
- Apps SDK component not rendering.

### `docs/architecture-decisions.md`

Record material deviations from this plan with:

- decision,
- alternatives,
- reason,
- consequences.

---

# 16. Completion gate

Do **not** claim the project is complete merely because the UI launches.

Completion requires all applicable checks below to pass.

## Core

- [ ] Add and switch workspaces.
- [ ] Workspace sandbox enforcement.
- [ ] Read/search/list.
- [ ] Safe patch/create.
- [ ] Revision conflict protection.
- [ ] Permission profiles.
- [ ] Pause access.
- [ ] Audit log.

## Git/process

- [ ] Git status/diff/log.
- [ ] Build/test allowed tasks.
- [ ] Arbitrary shell denied by default.
- [ ] timeout/output limits.

## MCP

- [ ] tools discover correctly.
- [ ] schemas validated.
- [ ] effective capabilities visible.
- [ ] disabled actions fail closed.
- [ ] write operation touches actual local file.

## Tunnel/ChatGPT

When eligible account access is available:

- [ ] private MCP reachable through current Secure MCP Tunnel.
- [ ] no public MCP exposure required.
- [ ] ChatGPT Web can invoke read tools.
- [ ] ChatGPT Web can invoke write/modify tools under current Full MCP rules.
- [ ] confirmation/approval behavior is documented accurately.
- [ ] disconnect/pause behavior verified.

If account/tunnel access is unavailable to the coding environment, clearly mark these as **external integration verification pending**; do not fake success.

## Security

- [ ] traversal tests.
- [ ] symlink/junction/reparse tests.
- [ ] sensitive-file tests.
- [ ] permission bypass tests.
- [ ] process injection tests.
- [ ] temporary grant tests.
- [ ] kill-switch tests.

## Quality

- [ ] build passes.
- [ ] relevant tests pass.
- [ ] formatter/linter passes.
- [ ] no obvious secrets in repo/logs.
- [ ] no dead experimental tunnel code.
- [ ] docs match actual implementation.

---

# 17. Final audit instruction for Codex

After implementation:

1. Review the entire diff, not only the last task.
2. Trace these flows end-to-end:
   - ChatGPT tool → policy → filesystem read.
   - ChatGPT tool → policy → patch → file.
   - ChatGPT tool → process runner → output.
   - local permission toggle → immediate enforcement.
   - pause → denial.
   - temporary grant → expiry/use-up.
3. Search for any direct filesystem/process call from MCP/UI that bypasses services/policy.
4. Search for naive path prefix checks.
5. Search for raw shell execution.
6. Search for unrestricted absolute path exposure.
7. Search for credentials/tokens in code and logs.
8. Re-run security-focused tests.
9. Re-run build/lint/format.
10. Update docs to match what actually shipped.
11. Report:
    - implemented features,
    - security guarantees,
    - known limitations,
    - external verification still required,
    - exact test/build commands and results.

Do not hide failed tests or unverified tunnel assumptions.

---

# 18. Suggested execution order for Codex

Use this order unless repository discovery gives a strong reason to change it:

```text
Phase 0   Discovery + official docs + MCP/tunnel spike
   ↓
Phase 1   App foundation
   ↓
Phase 2   Workspace Manager
   ↓
Phase 3   Policy + sandbox
   ↓
Phase 4   Read/search
   ↓
Phase 5   Editing
   ↓
Phase 6   Git
   ↓
Phase 7   Process runner
   ↓
Phase 8   Temporary approvals
   ↓
Phase 9   MCP tool layer
   ↓
Phase 10  Secure MCP Tunnel
   ↓
Phase 11  Local UX completion
   ↓
Phase 13  ChatGPT connection/setup verification
   ↓
Phase 14  Security hardening
   ↓
Phase 15  Packaging
   ↓
Phase 12  Apps SDK embedded UI (can move earlier if integration requires it)
   ↓
Final audit
```

The embedded ChatGPT UI is intentionally not on the critical path for the first secure working end-to-end local-file workflow.

---

# 19. Desired first milestone

The first milestone Codex should pursue is intentionally small:

> **ChatGPT Web can securely read a file from one locally approved test workspace through the supported Secure MCP Tunnel path, while Local Manager can pause access and the policy engine can prove a traversal attempt is blocked.**

Do not begin with a polished dashboard.

A successful first milestone demonstrates the hardest architectural assumptions:

```text
ChatGPT Web
   ↓
Custom MCP App
   ↓
Secure MCP Tunnel
   ↓
Local MCP
   ↓
Policy Engine
   ↓
Approved test workspace
```

Once this path is proven, expand to search → patch → Git diff → test runner → richer UI.

---

# 20. Codex kickoff command

Use the following as the immediate instruction after supplying this file:

> Read this entire plan, then inspect the current repository and official OpenAI/MCP documentation. Start with Phase 0. Do not assume Secure MCP Tunnel commands or MCP SDK behavior from memory. Choose the simplest architecture that satisfies the security boundaries. Build and verify the smallest end-to-end read-only connection first, document architectural decisions, then proceed phase-by-phase. Make reasonable implementation decisions without repeatedly asking me; stop only for genuine external credentials/account blockers or a material product decision. Keep testing focused on core/security behavior and always run a final cross-cutting audit before declaring completion.

---

# 21. Official references to verify during implementation

These links are starting points only. Re-check their latest content at execution time.

- OpenAI — Developer mode and MCP apps in ChatGPT:  
  https://help.openai.com/en/articles/12584461-developer-mode-and-full-mcp-connectors-in-chatgpt

- OpenAI — Build with the Apps SDK:  
  https://help.openai.com/en/articles/12515353-build-with-the-apps-sdk

- OpenAI Developer docs:  
  https://developers.openai.com/

- Model Context Protocol official documentation:  
  https://modelcontextprotocol.io/

---

## Definition of success

The finished product should feel like this:

```text
Local MCP Manager                         ChatGPT Web
─────────────────                         ───────────

● Connected                              Local Workspace MCP ●

Workspace                                Active:
Meeting Support        <──────────────── Meeting Support

Read/Search       ON                     Read/Search     ✓
Modify/Create     ON     Full MCP         Modify/Create   ✓
Delete            OFF   + Tunnel          Delete          ✕
Build/Test        ON  <───────────────>  Build/Test      ✓
Shell             OFF                     Shell           ✕

[ PAUSE CHATGPT ACCESS ]

Activity:
READ    src/...
SEARCH  speaker
PATCH   src/...
TEST    cargo test ✓
```

The key property is not “ChatGPT has filesystem access.”

The key property is:

> **ChatGPT receives narrowly scoped, revocable, auditable capabilities over explicitly approved local workspaces, while the local machine remains the final authority.**
