# Control Center + Auto Tunnel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn Local Workspace MCP Manager into a complete control-center UX with a Start/Stop connection lifecycle, one-time automatic Secure MCP Tunnel creation, persistent tunnel reuse, real readiness, safer secrets, and upgraded workspace/activity/security/settings pages.

**Architecture:** Keep `crates/core` as the filesystem security kernel. Add connection orchestration in Tauri (`connection.rs`) above a low-level tunnel process adapter (`tunnel.rs`), split non-secret settings from protected secrets, and expose structured snapshots/events to a new React navigation shell. Normal Start validates/reuses a saved tunnel; provisioning happens only when no valid tunnel exists.

**Tech Stack:** Rust 2021, Tauri 2, rmcp 3.1, axum 0.8, tokio, rusqlite, React 18, TypeScript 5, Vite 5, OpenAI tunnel-client CLI.

**Spec:** `docs/superpowers/specs/2026-08-24-control-center-auto-tunnel-design.md`

## Global Constraints

- Windows-first desktop application.
- MCP server remains bound to `127.0.0.1` only.
- Reuse an existing saved tunnel ID on normal Start whenever valid.
- `READY` requires tunnel-client `/readyz` HTTP 200; process spawn alone is insufficient.
- Admin key and runtime key are separate and never returned to the frontend.
- Existing workspace confinement, capability gates, sensitive deny list, optimistic concurrency and append-only audit must remain intact.
- No public Cloudflare Quick Tunnel UX.

---

### Task 1: Connection settings model and migration

**Files:**
- Modify: `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Produces `ConnectionSettings { tunnel_id, organization_id, workspace_id, tunnel_name, auto_start }`.
- Produces `SettingsSummary` safe for frontend.
- Produces migration hook for legacy `tunnel_api_key`.

- [ ] Add tests first for defaults, scope validation, roundtrip, and legacy JSON parsing/migration behavior.
- [ ] Implement non-secret settings and atomic save.
- [ ] Keep compatibility with an existing `tunnel_id` field.
- [ ] Verify settings tests.

### Task 2: Protected secret store

**Files:**
- Create: `src-tauri/src/secrets.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- `SecretStore::load_runtime_key()`
- `SecretStore::load_admin_key()`
- `SecretStore::save(runtime: Option<&str>, admin: Option<&str>)`
- `SecretStore::summary() -> SecretSummary`

- [ ] Write tests that secret summary exposes only booleans and that serialized secret storage is not plaintext.
- [ ] Implement Windows DPAPI-backed storage with a deterministic non-Windows test/fallback implementation guarded by cfg.
- [ ] Add legacy runtime-key import helper.
- [ ] Verify secret-store tests.

### Task 3: Tunnel CLI adapter and readiness surface

**Files:**
- Modify: `src-tauri/src/tunnel.rs`

**Interfaces:**
- `create_tunnel(...) -> TunnelRecord`
- `get_tunnel(...) -> TunnelRecord`
- `spawn_runtime(...) -> RuntimeProcess`
- `stop()`
- health URL file support.

- [ ] Add tests first for create/get/run argument construction, secret absence from argv, and JSON parsing.
- [ ] Add `admin --json tunnels create` with `OPENAI_ADMIN_KEY` environment injection.
- [ ] Add `admin --json tunnels get` using runtime key.
- [ ] Add runtime flags `--health.listen-addr=127.0.0.1:0` and `--health.url-file=<path>`.
- [ ] Drain/capture stdout/stderr rather than leaving undrained pipes.
- [ ] Verify tunnel unit tests.

### Task 4: Connection state machine

**Files:**
- Create: `src-tauri/src/connection.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- `ConnectionState` enum.
- `ConnectionSnapshot` struct.
- `ConnectionManager::start(...)`, `stop(...)`, `regenerate(...)`, `snapshot()`.
- Emits `connection-status` Tauri event.

- [ ] Add state-transition tests before implementation.
- [ ] Implement idempotent start/stop locking.
- [ ] Reuse saved tunnel path: `get` then runtime spawn.
- [ ] Provision path: create once, persist tunnel ID, then runtime spawn.
- [ ] Poll the health URL file and `/readyz` with a bounded timeout and provisioning-aware message.
- [ ] Error states preserve actionable code/detail without leaking secrets.
- [ ] Verify state-machine tests.

### Task 5: Tauri command surface

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- `connection_status`
- `start_connection`
- `stop_connection`
- `save_connection_settings`
- `connection_settings_summary`
- `regenerate_tunnel`

- [ ] Add serialization/validation tests where possible.
- [ ] Implement safe command DTOs; no key read-back endpoint.
- [ ] Register commands and managed states.
- [ ] Remove frontend dependency on old configure/start tunnel commands while preserving migration compatibility internally.

### Task 6: Audit metadata improvements

**Files:**
- Modify: `crates/core/src/pipeline.rs`
- Modify: `crates/core/src/ops/mod.rs`
- Modify: `src-tauri/src/server.rs`
- Test: core existing test modules and `src-tauri/tests/mcp_integration.rs`

**Interfaces:**
- Audit receives bounded operation context without contents/secrets.

- [ ] Add failing tests for MCP path metadata on read/create/patch and no sensitive-content leakage.
- [ ] Introduce an audited context/result mechanism with requested relative path/detail.
- [ ] Keep exactly-one-row-per-handler invocation behavior.
- [ ] Verify core and MCP integration tests.

### Task 7: Frontend API and app shell

**Files:**
- Modify: `src/api.ts`
- Replace: `src/App.tsx`
- Modify: `src/styles.css`
- Create: `src/components/AppShell.tsx`
- Create: `src/components/Sidebar.tsx`
- Create: `src/components/ConnectionHero.tsx`
- Create: `src/components/StatCard.tsx`

**Interfaces:**
- Typed `ConnectionSnapshot`, `ConnectionSettingsSummary`.
- Navigation page union: dashboard/workspaces/activity/security/settings.

- [ ] Define frontend types against backend DTOs.
- [ ] Build persistent sidebar + page header shell.
- [ ] Dashboard hero handles stopped/starting/needs_setup/ready/error states.
- [ ] Start/Stop is the dominant action.
- [ ] Tunnel ID Copy and local MCP URL are shown only when meaningful.

### Task 8: Workspaces, Activity and Security pages

**Files:**
- Replace/modify: `src/components/WorkspaceList.tsx`
- Replace/modify: `src/components/AuditTable.tsx`
- Replace/modify: `src/components/PauseButton.tsx`
- Create: `src/components/WorkspacePage.tsx`
- Create: `src/components/ActivityPage.tsx`
- Create: `src/components/SecurityPage.tsx`

- [ ] Upgrade workspace cards/toggles/add/remove UX.
- [ ] Activity view renders decision/error/path cleanly.
- [ ] Security page explains enforced controls and owns global Pause/Resume action.
- [ ] Add empty, busy and error states.

### Task 9: Connection Settings page

**Files:**
- Replace/retire: `src/components/SettingsDialog.tsx`
- Create: `src/components/SettingsPage.tsx`

**Interfaces:**
- Save replace-only runtime/admin keys.
- Scope fields: organization ID and workspace ID.
- Tunnel ID display + regenerate action.

- [ ] Build settings form with no secret prefill.
- [ ] Show `Configured` badges from backend boolean summaries.
- [ ] Validate at least one scope when provisioning is required.
- [ ] Add explicit Regenerate confirmation and explain connector-impact.

### Task 10: UX polish and responsive states

**Files:**
- Modify: `src/styles.css`
- Modify relevant components from Tasks 7–9.

- [ ] Consolidate visual tokens, typography, spacing, focus rings, semantic badges and buttons.
- [ ] Remove most inline styles.
- [ ] Ensure 900×700 layout works without horizontal clipping.
- [ ] Ensure keyboard focus and disabled/busy states are visible.

### Task 11: Documentation and compatibility

**Files:**
- Modify: `README.md`

- [ ] Correct MCP tool count to 8 including `list_workspaces`.
- [ ] Document first-run Start, admin/runtime key split, scope requirements and persistent tunnel reuse.
- [ ] Document readiness semantics and current lifecycle limitations.
- [ ] Remove old manual `Tunnel settings → Save & connect` instructions.

### Task 12: Verification and final audit

**Files:**
- Review all modified files.

- [ ] Run `cargo test -p core`.
- [ ] Run `cargo test -p local-workspace-mcp-manager`.
- [ ] Run `npm run build`.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo clippy --workspace --all-targets -- -D warnings` where environment permits.
- [ ] Static secret scan for `tunnel_api_key`, argv secret interpolation and key-return DTOs.
- [ ] Verify Start/Stop/reuse flow manually against a configured OpenAI tunnel where credentials are available.
- [ ] Re-read spec and confirm every acceptance criterion is either verified or explicitly reported as environment-dependent.
