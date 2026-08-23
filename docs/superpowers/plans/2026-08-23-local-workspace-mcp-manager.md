# Local Workspace MCP Manager Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Windows desktop app (Tauri 2 + Rust) that lets ChatGPT Web read/search/list/edit files in approved local folders via a loopback MCP server bridged by OpenAI's Secure MCP Tunnel.

**Architecture:** Single-process Rust app. A `core` crate holds all domain logic (SQLite persistence, policy engine, path confinement, file ops) as pure, testable functions. A thin `src-tauri` layer mounts the rmcp MCP server (Streamable HTTP on `127.0.0.1`), manages the `tunnel-client` sidecar process, and exposes Tauri commands to a single-screen React UI. Exactly one doorway to the filesystem: the policy pipeline.

**Tech Stack:** Tauri 2, rmcp 3.1 (`transport-streamable-http-server`), axum, tokio, rusqlite (bundled), serde/serde_json, dunce, sha2, regex, globset, ignore, uuid, thiserror, time; React 18 + TypeScript + Vite; openai/tunnel-client v0.0.12 as sidecar binary.

**Spec:** `docs/superpowers/specs/2026-08-23-local-workspace-mcp-manager-design.md` — read it first; this plan argues from it.

## Global Constraints

- Windows-first; all server binds are `127.0.0.1` ONLY. Never `0.0.0.0` (Windows Firewall prompt).
- MCP auth is `noauth` for MVP. No API keys, no OAuth in this plan.
- `audit_events` is append-only: no UPDATE/DELETE anywhere; enforced by SQL triggers.
- Every MCP tool request writes exactly one audit row (allow/deny/error) before its response is sent.
- Error codes are the exact stable strings from spec §6: `ACCESS_PAUSED`, `WORKSPACE_NOT_FOUND`, `READ_DISABLED`, `WRITE_DISABLED`, `PATH_OUTSIDE_WORKSPACE`, `SENSITIVE_FILE_BLOCKED`, `FILE_NOT_FOUND`, `FILE_EXISTS`, `INVALID_PATCH`, `FILE_CHANGED_SINCE_READ`, `RESULT_TOO_LARGE`, `IO_ERROR`.
- Result bounds (spec §3.2): read_file ≤ 262 144 bytes (256 KB); read_files ≤ 8 files; search_text ≤ 200 matches; list_directory ≤ 5 000 entries; find_files ≤ 500 paths; create_file ≤ 10 MB.
- Sensitive deny-list (spec §3.2): `.env`, `.env.*`, `*.pem`, `*.key`, `id_rsa*`, `.git/config`, `credentials`, `credentials.*`, `*.pfx`, `*.p12`, `secrets.*`; whole-dir block: `node_modules` at any depth.
- English UI and docs. All timestamps ISO 8601 UTC.
- Coverage target ≥ 80% on `crates/core` (checked at Task 15).
- Repo is not yet a git repo — Task 1 initializes it. Commit after every task; conventional commit messages (`feat:`, `test:`, `chore:`), no attribution lines.
- Toolchain: Rust stable (≥1.80), Node ≥ 20, `cargo install tauri-cli --version '^2'` (provides `cargo tauri`).

---

### Task 1: Workspace scaffold — git, cargo workspace, Tauri app with axum `/mcp` echo tool

**Files:**
- Create: `.gitignore`, `Cargo.toml` (workspace root), `crates/core/Cargo.toml`, `crates/core/src/lib.rs`
- Create: `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `src-tauri/build.rs`, `src-tauri/src/main.rs`, `src-tauri/src/server.rs`
- Create: `src/main.tsx`, `src/App.tsx`, `index.html`, `package.json`, `tsconfig.json`, `vite.config.ts`

**Interfaces:**
- Produces: cargo workspace with members `crates/core`, `src-tauri`. `core` is an empty lib other tasks fill. `server.rs` exposes `pub async fn serve(core: Arc<CoreServices>) -> std::io::Result<u16>` — for now it mounts one echo tool; later tasks extend `CoreServices`. A placeholder `pub struct CoreServices;` lives in `core` and gains fields in Tasks 2–8.

- [ ] **Step 1: Init git and scaffold files**

```powershell
cd D:\MCP-GPTWeb
git init
```

`.gitignore`:
```
/target
node_modules
dist
*.db
*.db-wal
*.db-shm
settings.json
```

Root `Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/core", "src-tauri"]

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
thiserror = "2"
uuid = { version = "1", features = ["v4"] }
time = { version = "0.3", features = ["formatting", "macros"] }
tempfile = "3"

[profile.dev]
opt-level = 1
```

`crates/core/Cargo.toml`:
```toml
[package]
name = "core"
version = "0.1.0"
edition = "2021"

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
uuid.workspace = true
time.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

`crates/core/src/lib.rs`:
```rust
/// Shared services handed to both the MCP server and Tauri commands.
/// Gains fields in later tasks (db, pause, audit).
#[derive(Default)]
pub struct CoreServices;

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_compiles() {
        assert!(true);
    }
}
```

- [ ] **Step 2: Create the Tauri app skeleton**

`src-tauri/Cargo.toml`:
```toml
[package]
name = "local-workspace-mcp-manager"
version = "0.1.0"
edition = "2021"

[dependencies]
core = { path = "../crates/core" }
tauri = { version = "2", features = [] }
tauri-plugin-dialog = "2"
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
axum = "0.8"
rmcp = { version = "3.1", features = ["server", "transport-streamable-http-server", "macros"] }

[build-dependencies]
tauri-build = { version = "2", features = [] }
```

`src-tauri/build.rs`:
```rust
fn main() { tauri_build::build() }
```

`src-tauri/tauri.conf.json`:
```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Local Workspace MCP Manager",
  "version": "0.1.0",
  "identifier": "com.local.workspace-mcp-manager",
  "build": {
    "beforeDevCommand": "npm run dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "npm run build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [{ "title": "Local Workspace MCP Manager", "width": 900, "height": 700 }],
    "security": { "csp": null }
  },
  "bundle": { "active": true, "targets": ["nsis"] }
}
```

`src-tauri/src/server.rs` (echo tool only — proves the rmcp/axum mount):
```rust
use std::sync::Arc;
use axum::Router;
use rmcp::{
    ServerHandler, ServiceExt, tool, tool_handler, tool_router,
    handler::server::router::tool::ToolRouter,
    model::{CallToolResult, Content, ServerInfo, ProtocolVersion, ServerCapabilities},
    transport::streamable_http_server::{
        StreamableHttpService, session::local::LocalSessionManager,
    },
};
use core::CoreServices;

#[derive(Clone)]
pub struct EchoServer {
    _core: Arc<CoreServices>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl EchoServer {
    pub fn new(core: Arc<CoreServices>) -> Self {
        Self { _core: core, tool_router: Self::tool_router() }
    }

    #[tool(description = "Echoes its input back. Scaffold health check.")]
    fn echo(&self, #[tool(aggr)] EchoArgs args) -> CallToolResult {
        CallToolResult::success(vec![Content::text(args.text)])
    }
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct EchoArgs {
    /// Text to echo back
    pub text: String,
}

#[tool_handler]
impl ServerHandler for EchoServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: rmcp::model::Implementation::from_build_env(),
            instructions: Some("Local workspace file access. Tools are scoped to approved workspaces.".into()),
        }
    }
}

/// Bind axum to a free loopback port and serve /mcp. Returns the chosen port.
pub async fn serve(core: Arc<CoreServices>) -> std::io::Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let service = StreamableHttpService::new(
        move || Ok(EchoServer::new(core.clone())),
        LocalSessionManager::default().into(),
        Default::default(),
    );
    let app = Router::new().nest_service("/mcp", service);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("mcp server error: {e}");
        }
    });
    Ok(port)
}
```

`src-tauri` needs `schemars` for the `JsonSchema` derive — add to its `[dependencies]`: `schemars = "1"`. (rmcp 3.1 uses schemars 1.x; if the macro errors on version mismatch, match whatever `rmcp` re-exports: `schemars = { version = "1", features = [] }`.)

`src-tauri/src/main.rs`:
```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod server;

#[tokio::main]
async fn main() {
    let core = std::sync::Arc::new(core::CoreServices);
    let port = server::serve(core.clone()).await.expect("bind loopback");
    println!("mcp server on 127.0.0.1:{port}/mcp");
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("tauri run");
}
```

Minimal frontend — `package.json`:
```json
{
  "name": "local-workspace-mcp-manager",
  "private": true,
  "type": "module",
  "scripts": { "dev": "vite", "build": "tsc && vite build" },
  "dependencies": { "@tauri-apps/api": "^2", "@tauri-apps/plugin-dialog": "^2", "react": "^18", "react-dom": "^18" },
  "devDependencies": { "@types/react": "^18", "@types/react-dom": "^18", "@vitejs/plugin-react": "^4", "typescript": "^5", "vite": "^5" }
}
```

`vite.config.ts`:
```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
});
```

`tsconfig.json`:
```json
{
  "compilerOptions": {
    "target": "ES2021", "lib": ["DOM", "ES2021"], "module": "ESNext",
    "moduleResolution": "bundler", "jsx": "react-jsx", "strict": true,
    "noEmit": true, "skipLibCheck": true
  },
  "include": ["src"]
}
```

`index.html`:
```html
<!doctype html>
<html lang="en">
  <head><meta charset="UTF-8" /><title>Local Workspace MCP Manager</title></head>
  <body><div id="root"></div><script type="module" src="/src/main.tsx"></script></body>
</html>
```

`src/main.tsx`:
```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode><App /></React.StrictMode>
);
```

`src/App.tsx`:
```tsx
export default function App() {
  return <div style={{ padding: 16 }}>Local Workspace MCP Manager — scaffold</div>;
}
```

- [ ] **Step 3: Install frontend deps and verify builds**

Run: `npm install` then `cargo build` at repo root.
Expected: both succeed. First build takes several minutes (Tauri + WebView2 toolchain).

- [ ] **Step 4: Verify the MCP endpoint with curl**

Run: `cargo run -p local-workspace-mcp-manager` in one shell; note the printed port. In another:

```powershell
$body = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
Invoke-WebRequest -Uri "http://127.0.0.1:PORT/mcp" -Method Post -Body $body -ContentType "application/json" -Headers @{ Accept = "application/json, text/event-stream" } | Select-Object -ExpandProperty Content
```

Expected: JSON (or SSE stream containing) a `result` with `serverInfo` and an `mcp-session-id` response header. If the protocol version is rejected, echo back the version the server advertises in the error and retry.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "chore: scaffold tauri workspace with rmcp echo server on loopback"
```

---

### Task 2: Error codes and domain models

**Files:**
- Create: `crates/core/src/error.rs`, `crates/core/src/models.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub enum ErrorCode` with `pub fn as_str(&self) -> &'static str` (all 12 codes from Global Constraints).
  - `pub struct CoreError { pub code: ErrorCode, pub message: String, pub detail: Option<serde_json::Value> }` with `CoreError::new(code, message)`, `CoreError::io(e: std::io::Error)`, and `impl std::fmt::Display`.
  - `pub struct Workspace { pub id: String, pub name: String, pub root_path: PathBuf, pub can_read: bool, pub can_write: bool, pub created_at: String, pub updated_at: String }` (all `Serialize + Deserialize + Clone + Debug`).
  - `pub struct AuditEvent` fields per spec §3.4: `ts, source, tool, workspace_id, op, decision, error_code, path, detail` (+ db-assigned `id` on read-back).

- [ ] **Step 1: Write failing tests**

`crates/core/src/error.rs` (tests inline at bottom):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable_strings() {
        assert_eq!(ErrorCode::AccessPaused.as_str(), "ACCESS_PAUSED");
        assert_eq!(ErrorCode::WorkspaceNotFound.as_str(), "WORKSPACE_NOT_FOUND");
        assert_eq!(ErrorCode::ReadDisabled.as_str(), "READ_DISABLED");
        assert_eq!(ErrorCode::WriteDisabled.as_str(), "WRITE_DISABLED");
        assert_eq!(ErrorCode::PathOutsideWorkspace.as_str(), "PATH_OUTSIDE_WORKSPACE");
        assert_eq!(ErrorCode::SensitiveFileBlocked.as_str(), "SENSITIVE_FILE_BLOCKED");
        assert_eq!(ErrorCode::FileNotFound.as_str(), "FILE_NOT_FOUND");
        assert_eq!(ErrorCode::FileExists.as_str(), "FILE_EXISTS");
        assert_eq!(ErrorCode::InvalidPatch.as_str(), "INVALID_PATCH");
        assert_eq!(ErrorCode::FileChangedSinceRead.as_str(), "FILE_CHANGED_SINCE_READ");
        assert_eq!(ErrorCode::ResultTooLarge.as_str(), "RESULT_TOO_LARGE");
        assert_eq!(ErrorCode::IoError.as_str(), "IO_ERROR");
    }

    #[test]
    fn error_displays_as_code_message() {
        let e = CoreError::new(ErrorCode::FileNotFound, "missing.txt not found");
        assert_eq!(e.to_string(), "FILE_NOT_FOUND: missing.txt not found");
    }

    #[test]
    fn io_error_maps_to_io_error_code() {
        let e = CoreError::io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"));
        assert_eq!(e.code, ErrorCode::IoError);
    }
}
```

`crates/core/src/models.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_serializes() {
        let ws = Workspace {
            id: "w1".into(), name: "Docs".into(),
            root_path: std::path::PathBuf::from("C:\\proj"),
            can_read: true, can_write: false,
            created_at: "2026-08-23T00:00:00Z".into(), updated_at: "2026-08-23T00:00:00Z".into(),
        };
        let v = serde_json::to_value(&ws).unwrap();
        assert_eq!(v["can_write"], serde_json::Value::Bool(false));
        let back: Workspace = serde_json::from_value(v).unwrap();
        assert_eq!(back.name, "Docs");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p core`
Expected: compile errors — `ErrorCode`, `CoreError`, `Workspace` not found.

- [ ] **Step 3: Implement**

`crates/core/src/error.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    AccessPaused,
    WorkspaceNotFound,
    ReadDisabled,
    WriteDisabled,
    PathOutsideWorkspace,
    SensitiveFileBlocked,
    FileNotFound,
    FileExists,
    InvalidPatch,
    FileChangedSinceRead,
    ResultTooLarge,
    IoError,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AccessPaused => "ACCESS_PAUSED",
            Self::WorkspaceNotFound => "WORKSPACE_NOT_FOUND",
            Self::ReadDisabled => "READ_DISABLED",
            Self::WriteDisabled => "WRITE_DISABLED",
            Self::PathOutsideWorkspace => "PATH_OUTSIDE_WORKSPACE",
            Self::SensitiveFileBlocked => "SENSITIVE_FILE_BLOCKED",
            Self::FileNotFound => "FILE_NOT_FOUND",
            Self::FileExists => "FILE_EXISTS",
            Self::InvalidPatch => "INVALID_PATCH",
            Self::FileChangedSinceRead => "FILE_CHANGED_SINCE_READ",
            Self::ResultTooLarge => "RESULT_TOO_LARGE",
            Self::IoError => "IO_ERROR",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoreError {
    pub code: ErrorCode,
    pub message: String,
    pub detail: Option<serde_json::Value>,
}

impl CoreError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), detail: None }
    }

    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn io(e: std::io::Error) -> Self {
        Self::new(ErrorCode::IoError, e.to_string())
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for CoreError {}
```

`crates/core/src/models.rs`:
```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub root_path: PathBuf,
    pub can_read: bool,
    pub can_write: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    #[serde(default)]
    pub id: i64,
    pub ts: String,
    pub source: String,             // "mcp" | "ui"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    pub op: String,
    pub decision: String,           // "allow" | "deny" | "error"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}
```

`crates/core/src/lib.rs`:
```rust
pub mod error;
pub mod models;

/// Shared services handed to both the MCP server and Tauri commands.
#[derive(Default)]
pub struct CoreServices;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p core`
Expected: 4 tests PASS.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "feat: core error codes and domain models"
```

---

### Task 3: SQLite database, migrations, workspace repository

**Files:**
- Create: `crates/core/src/db.rs`, `crates/core/src/workspace_repo.rs`
- Modify: `crates/core/src/lib.rs`, `crates/core/Cargo.toml` (add rusqlite)

**Interfaces:**
- Consumes: `Workspace`, `CoreError` from Task 2.
- Produces:
  - `pub struct Database { conn: std::sync::Mutex<rusqlite::Connection> }` with `pub fn open(path: &std::path::Path) -> Result<Self, CoreError>` (WAL mode, foreign keys ON, idempotent migrations incl. append-only triggers on `audit_events`).
  - `Database::with_conn<T>(&self, f: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>) -> Result<T, CoreError>` for the repos.
  - `pub struct WorkspaceRepo<'a>` or free functions taking `&Database`: `insert_workspace(db, name, root_path) -> Result<Workspace, CoreError>`, `list_workspaces(db) -> Result<Vec<Workspace>, CoreError>`, `get_workspace(db, id) -> Result<Workspace, CoreError>` (err `WORKSPACE_NOT_FOUND`), `update_capabilities(db, id, can_read, can_write) -> Result<(), CoreError>`, `delete_workspace(db, id) -> Result<(), CoreError>`.
  - `session_grants` table created by migration with a permanent grant row inserted alongside each workspace (spec §3.3).

- [ ] **Step 1: Add dependency**

In `crates/core/Cargo.toml` `[dependencies]` add:
```toml
rusqlite = { version = "0.32", features = ["bundled"] }
```

In `crates/core/src/lib.rs` add `pub mod db; pub mod workspace_repo;`.

- [ ] **Step 2: Write failing tests**

`crates/core/src/workspace_repo.rs` (tests inline):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::error::ErrorCode;

    fn test_db() -> Database {
        let dir = tempfile::tempdir().unwrap();
        Database::open(&dir.path().join("test.db")).unwrap()
    }
    // NOTE: tempdir is dropped at end of scope, but Database holds an open
    // connection to the file — on Windows the file delete may fail. Keep the
    // tempdir alive: return (Database, TempDir) from a helper instead.
}
```

Use this helper shape in all DB tests:
```rust
    fn setup() -> (Database, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("test.db")).unwrap();
        (db, dir)
    }

    #[test]
    fn insert_and_get_workspace() {
        let (db, _dir) = setup();
        let ws = insert_workspace(&db, "Docs", r"C:\proj\docs").unwrap();
        assert!(!ws.id.is_empty());
        assert_eq!(ws.can_read, true);
        assert_eq!(ws.can_write, false);
        let got = get_workspace(&db, &ws.id).unwrap();
        assert_eq!(got.root_path, ws.root_path);
    }

    #[test]
    fn get_unknown_workspace_errors() {
        let (db, _dir) = setup();
        let e = get_workspace(&db, "nope").unwrap_err();
        assert_eq!(e.code, ErrorCode::WorkspaceNotFound);
    }

    #[test]
    fn duplicate_root_path_rejected() {
        let (db, _dir) = setup();
        insert_workspace(&db, "A", r"C:\same").unwrap();
        assert!(insert_workspace(&db, "B", r"C:\same").is_err());
    }

    #[test]
    fn update_capabilities_and_list() {
        let (db, _dir) = setup();
        let ws = insert_workspace(&db, "A", r"C:\a").unwrap();
        update_capabilities(&db, &ws.id, true, true).unwrap();
        let all = list_workspaces(&db).unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].can_write);
    }

    #[test]
    fn delete_workspace_keeps_audit_table_and_cascades_grants() {
        let (db, _dir) = setup();
        let ws = insert_workspace(&db, "A", r"C:\a").unwrap();
        delete_workspace(&db, &ws.id).unwrap();
        assert_eq!(list_workspaces(&db).unwrap().len(), 0);
        // grant cascade: inserting again must succeed (no orphan blocking UNIQUE)
        assert!(insert_workspace(&db, "A", r"C:\a").is_ok());
    }

    #[test]
    fn audit_events_is_append_only() {
        let (db, _dir) = setup();
        db.with_conn(|c| {
            c.execute("INSERT INTO audit_events (ts, source, op, decision) VALUES ('t','ui','op','allow')", [])
        }).unwrap();
        let upd = db.with_conn(|c|
            c.execute("UPDATE audit_events SET decision='deny'", []));
        assert!(upd.is_err(), "UPDATE on audit_events must be rejected");
        let del = db.with_conn(|c|
            c.execute("DELETE FROM audit_events", []));
        assert!(del.is_err(), "DELETE on audit_events must be rejected");
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p core`
Expected: compile errors — `Database` not found.

- [ ] **Step 4: Implement**

`crates/core/src/db.rs`:
```rust
use crate::error::CoreError;
use std::path::Path;
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<rusqlite::Connection>,
}

const MIGRATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS workspaces (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  root_path  TEXT NOT NULL UNIQUE,
  can_read   INTEGER NOT NULL DEFAULT 1,
  can_write  INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_grants (
  id           TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  granted_at   TEXT NOT NULL,
  expires_at   TEXT,
  reason       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_events (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  ts           TEXT NOT NULL,
  source       TEXT NOT NULL,
  tool         TEXT,
  workspace_id TEXT,
  op           TEXT NOT NULL,
  decision     TEXT NOT NULL,
  error_code   TEXT,
  path         TEXT,
  detail       TEXT
);

CREATE TRIGGER IF NOT EXISTS audit_no_update BEFORE UPDATE ON audit_events
BEGIN SELECT RAISE(ABORT, 'audit_events is append-only'); END;

CREATE TRIGGER IF NOT EXISTS audit_no_delete BEFORE DELETE ON audit_events
BEGIN SELECT RAISE(ABORT, 'audit_events is append-only'); END;
"#;

impl Database {
    pub fn open(path: &Path) -> Result<Self, CoreError> {
        let conn = rusqlite::Connection::open(path).map_err(CoreError::io)?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(CoreError::io)?;
        conn.pragma_update(None, "foreign_keys", "ON").map_err(CoreError::io)?;
        conn.execute_batch(MIGRATIONS).map_err(CoreError::io)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn with_conn<T>(&self, f: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>) -> Result<T, CoreError> {
        let conn = self.conn.lock().unwrap();
        f(&conn).map_err(CoreError::io)
    }
}
```

`crates/core/src/workspace_repo.rs`:
```rust
use crate::db::Database;
use crate::error::{CoreError, ErrorCode};
use crate::models::Workspace;
use std::path::PathBuf;

fn now_iso() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn row_to_ws(row: &rusqlite::Row) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: row.get("id")?,
        name: row.get("name")?,
        root_path: PathBuf::from(row.get::<_, String>("root_path")?),
        can_read: row.get::<_, i64>("can_read")? != 0,
        can_write: row.get::<_, i64>("can_write")? != 0,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn insert_workspace(db: &Database, name: &str, root_path: &str) -> Result<Workspace, CoreError> {
    let id = uuid::Uuid::new_v4().to_string();
    let ts = now_iso();
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO workspaces (id, name, root_path, can_read, can_write, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, 0, ?4, ?4)",
            rusqlite::params![id, name, root_path, ts],
        )?;
        c.execute(
            "INSERT INTO session_grants (id, workspace_id, granted_at, expires_at, reason)
             VALUES (?1, ?2, ?3, NULL, 'user added via UI')",
            rusqlite::params![uuid::Uuid::new_v4().to_string(), id, ts],
        )?;
        Ok(())
    })?;
    get_workspace(db, &id)
}

pub fn get_workspace(db: &Database, id: &str) -> Result<Workspace, CoreError> {
    db.with_conn(|c| {
        c.query_row("SELECT * FROM workspaces WHERE id = ?1", [id], row_to_ws)
            .map_err(|_| CoreError::new(ErrorCode::WorkspaceNotFound, format!("workspace {id} not found")))
    })
}

pub fn list_workspaces(db: &Database) -> Result<Vec<Workspace>, CoreError> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT * FROM workspaces ORDER BY created_at")?;
        let rows = stmt.query_map([], row_to_ws)?;
        rows.collect()
    })
    .map_err(CoreError::io)
}

pub fn update_capabilities(db: &Database, id: &str, can_read: bool, can_write: bool) -> Result<(), CoreError> {
    let n = db.with_conn(|c| {
        c.execute(
            "UPDATE workspaces SET can_read = ?1, can_write = ?2, updated_at = ?3 WHERE id = ?4",
            rusqlite::params![can_read as i64, can_write as i64, now_iso(), id],
        )
    })?;
    if n == 0 {
        return Err(CoreError::new(ErrorCode::WorkspaceNotFound, format!("workspace {id} not found")));
    }
    Ok(())
}

pub fn delete_workspace(db: &Database, id: &str) -> Result<(), CoreError> {
    let n = db.with_conn(|c| c.execute("DELETE FROM workspaces WHERE id = ?1", [id]))?;
    if n == 0 {
        return Err(CoreError::new(ErrorCode::WorkspaceNotFound, format!("workspace {id} not found")));
    }
    Ok(())
}
```

Add the tests from Step 2 to the same file. `lib.rs` gains `pub mod db; pub mod workspace_repo;`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p core`
Expected: all PASS (Task 2 + Task 3 tests).

- [ ] **Step 6: Commit**

```powershell
git add -A
git commit -m "feat: sqlite persistence with workspace repo and append-only audit schema"
```

---

### Task 4: Audit log

**Files:**
- Create: `crates/core/src/audit.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: `Database`, `AuditEvent`.
- Produces:
  - `pub fn record(db: &Database, ev: &AuditEvent) -> Result<(), CoreError>` — inserts; `detail` serialized as JSON string; `id` ignored on input.
  - `pub fn record_simple(db, source, tool, workspace_id, op, decision, error_code, path, detail) -> Result<(), CoreError>` — convenience that fills `ts` (UTC RFC 3339).
  - `pub fn list_recent(db, limit: u32) -> Result<Vec<AuditEvent>, CoreError>` — newest first, parses `detail` JSON back.

- [ ] **Step 1: Write failing tests** (inline in `audit.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn setup() -> (Database, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("t.db")).unwrap();
        (db, dir)
    }

    #[test]
    fn record_and_read_back() {
        let (db, _d) = setup();
        record_simple(&db, "mcp", Some("read_file"), Some("w1"), "read_file",
            "allow", None, Some(r"C:\w\f.txt"), Some(serde_json::json!({"bytes": 10}))).unwrap();
        let events = list_recent(&db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decision, "allow");
        assert_eq!(events[0].detail.unwrap()["bytes"], 10);
        assert!(!events[0].ts.is_empty());
    }

    #[test]
    fn newest_first_and_limit() {
        let (db, _d) = setup();
        for i in 0..5 {
            record_simple(&db, "mcp", None, None, &format!("op{i}"), "allow", None, None, None).unwrap();
        }
        let events = list_recent(&db, 3).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].op, "op4");
    }
}
```

- [ ] **Step 2: Run to verify fail** — `cargo test -p core` → compile error `audit` module missing.

- [ ] **Step 3: Implement**

```rust
use crate::db::Database;
use crate::error::CoreError;
use crate::models::AuditEvent;

fn now_iso() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
pub fn record_simple(
    db: &Database, source: &str, tool: Option<&str>, workspace_id: Option<&str>,
    op: &str, decision: &str, error_code: Option<&str>, path: Option<&str>,
    detail: Option<serde_json::Value>,
) -> Result<(), CoreError> {
    record(db, &AuditEvent {
        id: 0, ts: now_iso(), source: source.into(), tool: tool.map(Into::into),
        workspace_id: workspace_id.map(Into::into), op: op.into(), decision: decision.into(),
        error_code: error_code.map(Into::into), path: path.map(Into::into), detail,
    })
}

pub fn record(db: &Database, ev: &AuditEvent) -> Result<(), CoreError> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO audit_events (ts, source, tool, workspace_id, op, decision, error_code, path, detail)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![
                ev.ts, ev.source, ev.tool, ev.workspace_id, ev.op, ev.decision,
                ev.error_code, ev.path,
                ev.detail.as_ref().map(|d| d.to_string()),
            ],
        )?;
        Ok(())
    })
}

pub fn list_recent(db: &Database, limit: u32) -> Result<Vec<AuditEvent>, CoreError> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT * FROM audit_events ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit], |row| {
            Ok(AuditEvent {
                id: row.get("id")?,
                ts: row.get("ts")?,
                source: row.get("source")?,
                tool: row.get("tool")?,
                workspace_id: row.get("workspace_id")?,
                op: row.get("op")?,
                decision: row.get("decision")?,
                error_code: row.get("error_code")?,
                path: row.get("path")?,
                detail: row.get::<_, Option<String>>("detail")?
                    .and_then(|s| serde_json::from_str(&s).ok()),
            })
        })?;
        rows.collect()
    })
    .map_err(CoreError::io)
}
```

`lib.rs` gains `pub mod audit;`.

- [ ] **Step 4: Run to verify pass** — `cargo test -p core` → all PASS.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "feat: append-only audit log"
```

---

### Task 5: Path confinement (the critical security module)

**Files:**
- Create: `crates/core/src/confinement.rs`
- Modify: `crates/core/src/lib.rs`, `crates/core/Cargo.toml` (add `dunce = "1"`)

**Interfaces:**
- Produces:
  - `pub fn resolve(target: &std::path::Path) -> std::io::Result<std::path::PathBuf>` — full canonicalization; for non-existent targets, canonicalizes the nearest existing ancestor and appends the remaining components.
  - `pub fn ensure_inside(root: &Path, target: &Path) -> Result<PathBuf, CoreError>` — resolves `target`, verifies component-wise containment under canonical `root`; returns the resolved path or `PATH_OUTSIDE_WORKSPACE` (with `detail.requested` = original target string).
  - `fn starts_with_components(path: &Path, prefix: &Path) -> bool` (private).

- [ ] **Step 1: Write failing tests** (inline in `confinement.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use std::fs;

    fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("file.txt"), "hello").unwrap();
        (dir, root)
    }

    #[test]
    fn existing_file_inside_resolves() {
        let (_d, root) = setup();
        let p = ensure_inside(&root, &root.join("file.txt")).unwrap();
        assert!(p.ends_with("file.txt"));
    }

    #[test]
    fn traversal_escape_rejected() {
        let (_d, root) = setup();
        let e = ensure_inside(&root, &root.join("..").join("outside.txt")).unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideWorkspace);
    }

    #[test]
    fn absolute_path_outside_rejected() {
        let (_d, root) = setup();
        let e = ensure_inside(&root, std::path::Path::new(r"C:\Windows\system32\config.sys")).unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideWorkspace);
    }

    #[test]
    fn nonexistent_nested_path_inside_allowed() {
        let (_d, root) = setup();
        let p = ensure_inside(&root, &root.join("sub").join("new").join("f.txt")).unwrap();
        assert!(p.starts_with(&dunce::canonicalize(&root).unwrap()));
    }

    #[test]
    fn case_insensitive_windows_match() {
        let (_d, root) = setup();
        let upper = root.to_string_lossy().to_uppercase();
        let p = ensure_inside(&root, std::path::Path::new(&upper).join("file.txt").as_path()).unwrap();
        assert!(p.ends_with("file.txt"));
    }

    #[test]
    fn symlink_escape_rejected() {
        let (d, root) = setup();
        #[cfg(unix)]
        { std::os::unix::fs::symlink(d.path(), root.join("link")).unwrap(); }
        #[cfg(windows)]
        {
            // junction: mklink /J via cmd (no admin needed for junctions to dirs)
            let out = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J",
                    &root.join("link").to_string_lossy(),
                    &d.path().to_string_lossy()])
                .output().unwrap();
            assert!(out.status.success(), "junction creation failed: {:?}", out);
        }
        let e = ensure_inside(&root, &root.join("link").join("secret.txt")).unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideWorkspace);
    }

    #[test]
    fn root_itself_is_inside() {
        let (_d, root) = setup();
        let p = ensure_inside(&root, &root).unwrap();
        assert_eq!(p, dunce::canonicalize(&root).unwrap());
    }
}
```

- [ ] **Step 2: Run to verify fail** — `cargo test -p core` → compile error.

- [ ] **Step 3: Implement**

```rust
use crate::error::{CoreError, ErrorCode};
use std::path::{Path, PathBuf};

/// Canonicalize, following symlinks/junctions. For a target that does not
/// exist, canonicalize the nearest existing ancestor and re-append the
/// non-existent tail components.
pub fn resolve(target: &Path) -> std::io::Result<PathBuf> {
    if let Ok(p) = dunce::canonicalize(target) {
        return Ok(p);
    }
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    for anc in target.ancestors().skip(1) {
        match dunce::canonicalize(anc) {
            Ok(base) => {
                let mut resolved = base;
                for part in suffix.iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(_) => {
                if let Some(name) = anc.file_name() {
                    suffix.push(name.to_os_string());
                }
            }
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("no existing ancestor for {}", target.display()),
    ))
}

fn starts_with_components(path: &Path, prefix: &Path) -> bool {
    let mut a = path.components();
    let mut b = prefix.components();
    loop {
        match (b.next(), a.next()) {
            (None, _) => return true,
            (Some(pb), Some(pa)) if pa == pb => continue,
            _ => return false,
        }
    }
}

/// Resolve `target` and enforce that it stays inside canonical `root`.
pub fn ensure_inside(root: &Path, target: &Path) -> Result<PathBuf, CoreError> {
    let resolved = resolve(target).map_err(CoreError::io)?;
    let root_c = dunce::canonicalize(root).map_err(CoreError::io)?;
    if starts_with_components(&resolved, &root_c) {
        Ok(resolved)
    } else {
        Err(CoreError::new(
            ErrorCode::PathOutsideWorkspace,
            format!("{} resolves outside the workspace", target.display()),
        )
        .with_detail(serde_json::json!({ "requested": target.to_string_lossy() })))
    }
}
```

- [ ] **Step 4: Run to verify pass** — `cargo test -p core` → all PASS, including the junction test (run it in a real PowerShell, not a sandbox that blocks `mklink`).

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "feat: component-wise path confinement with junction/symlink escape detection"
```

---

### Task 6: Sensitive-file deny list

**Files:**
- Create: `crates/core/src/sensitive.rs`
- Modify: `crates/core/src/lib.rs`, `crates/core/Cargo.toml` (add `globset = "0.4"`)

**Interfaces:**
- Produces: `pub struct SensitiveMatcher` with `SensitiveMatcher::new()` and `pub fn is_blocked(&self, rel: &Path) -> bool` where `rel` is relative to the workspace root. Constants `pub const FILE_PATTERNS: &[&str]` and `pub const BLOCKED_DIR_NAMES: &[&str]` exactly as Global Constraints.

- [ ] **Step 1: Write failing tests** (inline)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn m() -> SensitiveMatcher { SensitiveMatcher::new() }

    #[test]
    fn env_files_blocked_at_any_depth() {
        assert!(m().is_blocked(Path::new(".env")));
        assert!(m().is_blocked(Path::new(".env.local")));
        assert!(m().is_blocked(Path::new("sub/dir/.env")));
    }

    #[test]
    fn lookalikes_allowed() {
        assert!(!m().is_blocked(Path::new("env.txt")));
        assert!(!m().is_blocked(Path::new("environment.ts")));
        assert!(!m().is_blocked(Path::new("src/keymaster.rs")));
    }

    #[test]
    fn key_material_blocked() {
        assert!(m().is_blocked(Path::new("server.pem")));
        assert!(m().is_blocked(Path::new("deep/a/b/id_rsa_backup")));
        assert!(m().is_blocked(Path::new("cert.p12")));
    }

    #[test]
    fn git_config_blocked_but_git_head_allowed() {
        assert!(m().is_blocked(Path::new(".git/config")));
        assert!(!m().is_blocked(Path::new(".git/HEAD")));
    }

    #[test]
    fn node_modules_blocked_entirely() {
        assert!(m().is_blocked(Path::new("node_modules")));
        assert!(m().is_blocked(Path::new("node_modules/pkg/index.js")));
        assert!(m().is_blocked(Path::new("web/node_modules/pkg/index.js")));
    }

    #[test]
    fn normal_files_allowed() {
        assert!(!m().is_blocked(Path::new("src/main.rs")));
        assert!(!m().is_blocked(Path::new("README.md")));
    }
}
```

- [ ] **Step 2: Run to verify fail.**

- [ ] **Step 3: Implement**

```rust
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;

pub const FILE_PATTERNS: &[&str] = &[
    "**/.env", "**/.env.*", "**/*.pem", "**/*.key", "**/id_rsa*",
    "**/.git/config", "**/credentials", "**/credentials.*",
    "**/*.pfx", "**/*.p12", "**/secrets.*",
];

pub const BLOCKED_DIR_NAMES: &[&str] = &["node_modules"];

pub struct SensitiveMatcher {
    files: GlobSet,
    dirs: GlobSet,
}

impl SensitiveMatcher {
    pub fn new() -> Self {
        let mut fb = GlobSetBuilder::new();
        for p in FILE_PATTERNS {
            fb.add(Glob::new(p).unwrap());
        }
        let mut db = GlobSetBuilder::new();
        for name in BLOCKED_DIR_NAMES {
            db.add(Glob::new(&format!("**/{name}")).unwrap());
        }
        Self { files: fb.build().unwrap(), dirs: db.build().unwrap() }
    }

    /// `rel` must be relative to the workspace root. Checks the full relative
    /// path AND every ancestor directory (so `node_modules/x/y` is caught by
    /// the `node_modules` dir rule).
    pub fn is_blocked(&self, rel: &Path) -> bool {
        for anc in rel.ancestors() {
            if anc.as_os_str().is_empty() {
                break;
            }
            if self.files.is_match(anc) || self.dirs.is_match(anc) {
                return true;
            }
        }
        false
    }
}

impl Default for SensitiveMatcher {
    fn default() -> Self { Self::new() }
}
```

- [ ] **Step 4: Run to verify pass** — `cargo test -p core` → all PASS.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "feat: sensitive-file deny list with glob matching"
```

---

### Task 7: Policy pipeline + kill switch

**Files:**
- Create: `crates/core/src/pause.rs`, `crates/core/src/pipeline.rs`
- Modify: `crates/core/src/lib.rs`, `crates/core/src/models.rs` (extend `CoreServices`)

**Interfaces:**
- Produces:
  - `pub struct PauseFlag(AtomicBool)` with `is_paused()`, `set(bool)`, `Default`.
  - `CoreServices` becomes `pub struct CoreServices { pub db: Database, pub pause: PauseFlag, pub sensitive: SensitiveMatcher }` with `CoreServices::open(db_path: &Path) -> Result<Self, CoreError>` (replaces the `Default` derive).
  - `pub fn run_tool<T>(core: &CoreServices, tool: &str, workspace_id: &str, needs_write: bool, f: impl FnOnce(&Workspace, &Path) -> Result<T, CoreError>) -> Result<T, CoreError>` — the fail-closed pipeline: pause check → workspace lookup → capability check → `f` (which does confinement + op validation + execution) → audit "allow". Deny/error audit rows are written by the caller wrapper `run_tool_audited` (same signature, returns `Result<T, CoreError>` but records deny/error rows itself). **All tool call sites use `run_tool_audited`.**

- [ ] **Step 1: Write failing tests** (inline in `pipeline.rs`; needs a temp workspace dir + db)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::error::ErrorCode;
    use crate::workspace_repo::{insert_workspace, update_capabilities};
    use std::fs;

    fn setup() -> (CoreServices, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("t.db")).unwrap();
        fs::create_dir_all(dir.path().join("ws")).unwrap();
        let ws = insert_workspace(&db, "W", &dir.path().join("ws").to_string_lossy()).unwrap();
        let core = CoreServices {
            db, pause: PauseFlag::default(), sensitive: crate::sensitive::SensitiveMatcher::new(),
        };
        (core, dir, ws.id)
    }

    #[test]
    fn paused_blocks_everything() {
        let (mut core, _d, ws_id) = setup();
        core.pause.set(true);
        let e = run_tool_audited(&core, "list_directory", &ws_id, false, |_ws, _root| Ok(1)).unwrap_err();
        assert_eq!(e.code, ErrorCode::AccessPaused);
        let events = crate::audit::list_recent(&core.db, 10).unwrap();
        assert_eq!(events[0].error_code.as_deref(), Some("ACCESS_PAUSED"));
        assert_eq!(events[0].decision, "deny");
    }

    #[test]
    fn unknown_workspace_denied_and_audited() {
        let (core, _d, _id) = setup();
        let e = run_tool_audited(&core, "read_file", "ghost", false, |_ws, _r| Ok(())).unwrap_err();
        assert_eq!(e.code, ErrorCode::WorkspaceNotFound);
    }

    #[test]
    fn write_needs_write_capability() {
        let (core, _d, ws_id) = setup(); // can_write defaults false
        let e = run_tool_audited(&core, "apply_patch", &ws_id, true, |_ws, _r| Ok(())).unwrap_err();
        assert_eq!(e.code, ErrorCode::WriteDisabled);
    }

    #[test]
    fn read_needs_read_capability() {
        let (core, _d, ws_id) = setup();
        update_capabilities(&core.db, &ws_id, false, false).unwrap();
        let e = run_tool_audited(&core, "read_file", &ws_id, false, |_ws, _r| Ok(())).unwrap_err();
        assert_eq!(e.code, ErrorCode::ReadDisabled);
    }

    #[test]
    fn success_audits_allow_with_workspace() {
        let (core, _d, ws_id) = setup();
        let v = run_tool_audited(&core, "read_file", &ws_id, false, |ws, root| {
            assert_eq!(ws.can_read, true);
            assert!(root.ends_with("ws"));
            Ok(42)
        }).unwrap();
        assert_eq!(v, 42);
        let events = crate::audit::list_recent(&core.db, 10).unwrap();
        assert_eq!(events[0].decision, "allow");
        assert_eq!(events[0].workspace_id.as_deref(), Some(ws_id.as_str()));
    }

    #[test]
    fn closure_error_audited_as_error() {
        let (core, _d, ws_id) = setup();
        let e = run_tool_audited(&core, "read_file", &ws_id, false,
            |_ws, _r| Err(CoreError::new(ErrorCode::FileNotFound, "x"))).unwrap_err();
        assert_eq!(e.code, ErrorCode::FileNotFound);
        let events = crate::audit::list_recent(&core.db, 10).unwrap();
        assert_eq!(events[0].decision, "error");
    }
}
```

- [ ] **Step 2: Run to verify fail.**

- [ ] **Step 3: Implement**

`pause.rs`:
```rust
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub struct PauseFlag(AtomicBool);

impl PauseFlag {
    pub fn is_paused(&self) -> bool { self.0.load(Ordering::SeqCst) }
    pub fn set(&self, paused: bool) { self.0.store(paused, Ordering::SeqCst) }
}
```

`pipeline.rs`:
```rust
use crate::audit;
use crate::error::{CoreError, ErrorCode};
use crate::models::Workspace;
use crate::pause::PauseFlag;
use crate::sensitive::SensitiveMatcher;
use crate::workspace_repo::get_workspace;
use crate::CoreServices;
use std::path::Path;

pub fn run_tool<T>(
    core: &CoreServices,
    tool: &str,
    workspace_id: &str,
    needs_write: bool,
    f: impl FnOnce(&Workspace, &Path) -> Result<T, CoreError>,
) -> Result<T, CoreError> {
    if core.pause.is_paused() {
        return Err(CoreError::new(ErrorCode::AccessPaused, "access is paused by the user"));
    }
    let ws = get_workspace(&core.db, workspace_id)?;
    if needs_write && !ws.can_write {
        return Err(CoreError::new(ErrorCode::WriteDisabled, format!("workspace '{}' has write access disabled", ws.name)));
    }
    if !needs_write && !ws.can_read {
        return Err(CoreError::new(ErrorCode::ReadDisabled, format!("workspace '{}' has read access disabled", ws.name)));
    }
    let root = ws.root_path.clone();
    let result = f(&ws, &root);
    audit::record_simple(
        &core.db, "mcp", Some(tool), Some(workspace_id), tool,
        if result.is_ok() { "allow" } else { "error" },
        result.as_ref().err().map(|e| e.code.as_str()),
        None, None,
    )?;
    result
}

/// Like `run_tool` but also audits pre-execution denies (pause, capability)
/// with decision "deny". This is the entry point every tool uses.
pub fn run_tool_audited<T>(
    core: &CoreServices,
    tool: &str,
    workspace_id: &str,
    needs_write: bool,
    f: impl FnOnce(&Workspace, &Path) -> Result<T, CoreError>,
) -> Result<T, CoreError> {
    let outcome = run_tool(core, tool, workspace_id, needs_write, f);
    if let Err(e) = &outcome {
        // run_tool already audited errors from inside the closure; re-auditing
        // would double-count. Distinguish: pause/capability errors happen
        // before the closure. run_tool audits everything as "error" though —
        // so instead we audit denies here and make run_tool NOT audit.
    }
    outcome
}
```

**Correction while implementing** — the two-layer audit is convoluted. Simpler, correct version: `run_tool_audited` is the ONLY auditor; `run_tool` does not exist. Single function:

```rust
pub fn run_tool_audited<T>(
    core: &CoreServices,
    tool: &str,
    workspace_id: &str,
    needs_write: bool,
    f: impl FnOnce(&Workspace, &Path) -> Result<T, CoreError>,
) -> Result<T, CoreError> {
    let pre = (|| -> Result<(Workspace, PathBuf), CoreError> {
        if core.pause.is_paused() {
            return Err(CoreError::new(ErrorCode::AccessPaused, "access is paused by the user"));
        }
        let ws = get_workspace(&core.db, workspace_id)?;
        if needs_write && !ws.can_write {
            return Err(CoreError::new(ErrorCode::WriteDisabled,
                format!("workspace '{}' has write access disabled", ws.name)));
        }
        if !ws.can_read {
            return Err(CoreError::new(ErrorCode::ReadDisabled,
                format!("workspace '{}' has read access disabled", ws.name)));
        }
        Ok((ws, ws.root_path.clone()))
    })();

    let (ws, root) = match pre {
        Ok(ok) => ok,
        Err(e) => {
            let decision = if e.code == ErrorCode::AccessPaused
                || matches!(e.code, ErrorCode::ReadDisabled | ErrorCode::WriteDisabled) { "deny" } else { "error" };
            audit::record_simple(&core.db, "mcp", Some(tool), None, tool,
                decision, Some(e.code.as_str()), None, None)?;
            return Err(e);
        }
    };

    let result = f(&ws, &root);
    audit::record_simple(&core.db, "mcp", Some(tool), Some(workspace_id), tool,
        if result.is_ok() { "allow" } else { "error" },
        result.as_ref().err().map(|e| e.code.as_str()), None, None)?;
    result
}
```

(Use this final version; drop the draft above it. `use std::path::PathBuf` accordingly. The test `paused_blocks_everything` asserts the deny row; `closure_error_audited_as_error` asserts exactly one "error" row — matching this version.)

Update `CoreServices` in `lib.rs`:
```rust
pub struct CoreServices {
    pub db: Database,
    pub pause: PauseFlag,
    pub sensitive: SensitiveMatcher,
}

impl CoreServices {
    pub fn open(db_path: &Path) -> Result<Self, CoreError> {
        Ok(Self {
            db: Database::open(db_path)?,
            pause: PauseFlag::default(),
            sensitive: SensitiveMatcher::new(),
        })
    }
}
```
Fix Task 1's `server.rs` construction site accordingly (`CoreServices::open(...)` — Task 12 wires the real path; for now point it at a temp path so the build stays green, or keep `serve` taking `Arc<CoreServices>` unchanged since it already does).

- [ ] **Step 4: Run to verify pass** — `cargo test -p core` → all PASS.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "feat: fail-closed policy pipeline with pause kill switch and full auditing"
```

---

### Task 8: Read ops — list_directory, find_files, search_text, read_file, read_files

**Files:**
- Create: `crates/core/src/ops/mod.rs`, `crates/core/src/ops/list_dir.rs`, `crates/core/src/ops/find_files.rs`, `crates/core/src/ops/search_text.rs`, `crates/core/src/ops/read_file.rs`
- Modify: `crates/core/src/lib.rs`, `crates/core/Cargo.toml` (add `ignore = "0.4"`, `regex = "1"`, `sha2 = "0.10"`, `walkdir = "2"`)

**Interfaces:**
- Consumes: `ensure_inside`, `SensitiveMatcher`, bounds constants.
- Produces (all take `root: &Path` already vetted by the pipeline; they run confinement themselves on the user-supplied relative path):
  - `ops::list_dir(core, ws_root, rel: &str, recursive: bool) -> Result<ListDirResult, CoreError>`; `ListDirResult { entries: Vec<DirEntry>, truncated: bool }`, `DirEntry { name: String, kind: "file"|"dir", size: u64, modified: Option<String> }`.
  - `ops::find_files(core, ws_root, pattern: &str) -> Result<FindResult, CoreError>`; `FindResult { paths: Vec<String>, truncated: bool }` (relative paths).
  - `ops::search_text(core, ws_root, query: &str, path_prefix: Option<&str>, case_sensitive: bool, is_regex: bool, max_results: Option<usize>) -> Result<SearchResult, CoreError>`; `SearchResult { matches: Vec<TextMatch>, truncated: bool }`, `TextMatch { path: String, line_no: u64, line: String, col: u64 }`.
  - `ops::read_file(core, ws_root, rel: &str, offset_line: Option<u64>, max_lines: Option<u64>) -> Result<ReadResult, CoreError>`; `ReadResult { content: String, revision: String, total_lines: u64, truncated: bool }`.
  - `ops::read_files(core, ws_root, rels: &[String]) -> Result<Vec<FileResult>, CoreError>`; `FileResult { path: String, result: Result<ReadResult, CoreError> }` — per-file, never fails the batch; >8 paths → `ResultTooLarge`.
  - Shared: `pub fn sha256_hex(bytes: &[u8]) -> String`; `pub const MAX_READ_BYTES: u64 = 262_144;` etc.
  - Every op: `ensure_inside` first, then sensitive check on the path relative to root (reads of blocked files → `SENSITIVE_FILE_BLOCKED`); directory listings skip (but note) sensitive entries rather than erroring.

- [ ] **Step 1: Write failing tests** (one `#[cfg(test)]` block per file; representative cases)

`ops/list_dir.rs` tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::run_tool_audited;
    use std::fs;

    fn setup() -> (crate::CoreServices, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Database::open(&dir.path().join("t.db")).unwrap();
        fs::create_dir_all(dir.path().join("ws/sub")).unwrap();
        fs::write(dir.path().join("ws/a.txt"), "hello").unwrap();
        fs::write(dir.path().join("ws/.env"), "SECRET=1").unwrap();
        let ws = crate::workspace_repo::insert_workspace(&db, "W", &dir.path().join("ws").to_string_lossy()).unwrap();
        let core = crate::CoreServices { db, pause: Default::default(), sensitive: crate::sensitive::SensitiveMatcher::new() };
        (core, dir, ws.id)
    }

    #[test]
    fn lists_entries_without_sensitive_content_leak() {
        let (core, d, ws) = setup();
        let r = run_tool_audited(&core, "list_directory", &ws, false, |_, root| {
            list_dir(&core, root, ".", false)
        }).unwrap();
        let names: Vec<&str> = r.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"sub"));
        // sensitive file is hidden from listings entirely (default-deny)
        assert!(!names.contains(&".env"));
        assert!(!r.truncated);
    }

    #[test]
    fn recursive_lists_nested() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/sub/b.txt"), "x").unwrap();
        let r = run_tool_audited(&core, "list_directory", &ws, false, |_, root| {
            list_dir(&core, root, ".", true)
        }).unwrap();
        let names: Vec<&str> = r.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"sub/b.txt"));
    }

    #[test]
    fn rel_outside_workspace_rejected() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "list_directory", &ws, false, |_, root| {
            list_dir(&core, root, "..", false)
        }).unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::PathOutsideWorkspace);
    }
}
```

`ops/read_file.rs` tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::run_tool_audited;
    use std::fs;

    fn setup() -> (crate::CoreServices, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Database::open(&dir.path().join("t.db")).unwrap();
        fs::create_dir_all(dir.path().join("ws")).unwrap();
        let ws = crate::workspace_repo::insert_workspace(&db, "W", &dir.path().join("ws").to_string_lossy()).unwrap();
        let core = crate::CoreServices { db, pause: Default::default(), sensitive: crate::sensitive::SensitiveMatcher::new() };
        (core, dir, ws.id)
    }

    #[test]
    fn reads_file_with_revision() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/f.txt"), "line1\nline2\n").unwrap();
        let r = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", None, None)
        }).unwrap();
        assert_eq!(r.content, "line1\nline2\n");
        assert_eq!(r.total_lines, 2);
        assert!(!r.truncated);
        // revision is sha256 of full content — stable across reads
        let again = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", None, None)
        }).unwrap();
        assert_eq!(r.revision, again.revision);
        assert_eq!(r.revision.len(), 64);
    }

    #[test]
    fn offset_and_max_lines_window() {
        let (core, d, ws) = setup();
        let content = (1..=10).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        fs::write(d.path().join("ws/f.txt"), content).unwrap();
        let r = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", Some(2), Some(3)) // 0-based offset
        }).unwrap();
        assert_eq!(r.content, "line3\nline4\nline5");
        assert!(r.truncated);
        assert_eq!(r.total_lines, 10);
    }

    #[test]
    fn big_file_truncated_at_byte_bound() {
        let (core, d, ws) = setup();
        let big = "x".repeat(300_000);
        fs::write(d.path().join("ws/big.txt"), &big).unwrap();
        let r = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "big.txt", None, None)
        }).unwrap();
        assert!(r.content.len() <= 262_144);
        assert!(r.truncated);
    }

    #[test]
    fn sensitive_file_blocked() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/.env"), "SECRET").unwrap();
        let e = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, ".env", None, None)
        }).unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::SensitiveFileBlocked);
    }

    #[test]
    fn missing_file_is_file_not_found() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "ghost.txt", None, None)
        }).unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::FileNotFound);
    }

    #[test]
    fn read_files_caps_at_8() {
        let (core, _d, ws) = setup();
        let paths: Vec<String> = (0..9).map(|i| format!("f{i}.txt")).collect();
        let e = run_tool_audited(&core, "read_files", &ws, false, |_, root| {
            read_files(&core, root, &paths)
        }).unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::ResultTooLarge);
    }

    #[test]
    fn read_files_per_file_results() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/ok.txt"), "fine").unwrap();
        let r = run_tool_audited(&core, "read_files", &ws, false, |_, root| {
            read_files(&core, root, &["ok.txt".into(), ".env".into(), "ghost.txt".into()])
        }).unwrap();
        assert!(r[0].result.is_ok());
        assert_eq!(r[1].result.as_ref().unwrap_err().code, crate::error::ErrorCode::SensitiveFileBlocked);
        assert_eq!(r[2].result.as_ref().unwrap_err().code, crate::error::ErrorCode::FileNotFound);
    }
}
```

`ops/find_files.rs` tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::run_tool_audited;
    use std::fs;

    fn setup() -> (crate::CoreServices, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Database::open(&dir.path().join("t.db")).unwrap();
        fs::create_dir_all(dir.path().join("ws/src")).unwrap();
        fs::write(dir.path().join("ws/src/main.rs"), "").unwrap();
        fs::write(dir.path().join("ws/README.md"), "").unwrap();
        let ws = crate::workspace_repo::insert_workspace(&db, "W", &dir.path().join("ws").to_string_lossy()).unwrap();
        let core = crate::CoreServices { db, pause: Default::default(), sensitive: crate::sensitive::SensitiveMatcher::new() };
        (core, dir, ws.id)
    }

    #[test]
    fn glob_finds_rs_files() {
        let (core, _d, ws) = setup();
        let r = run_tool_audited(&core, "find_files", &ws, false, |_, root| {
            find_files(&core, root, "**/*.rs")
        }).unwrap();
        assert_eq!(r.paths, vec!["src/main.rs".to_string()]);
    }

    #[test]
    fn node_modules_excluded() {
        let (core, d, ws) = setup();
        fs::create_dir_all(d.path().join("ws/node_modules/leftpad")).unwrap();
        fs::write(d.path().join("ws/node_modules/leftpad/index.js"), "").unwrap();
        let r = run_tool_audited(&core, "find_files", &ws, false, |_, root| {
            find_files(&core, root, "**/*.js")
        }).unwrap();
        assert!(r.paths.is_empty());
    }
}
```

`ops/search_text.rs` tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::run_tool_audited;
    use std::fs;

    fn setup() -> (crate::CoreServices, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Database::open(&dir.path().join("t.db")).unwrap();
        fs::create_dir_all(dir.path().join("ws")).unwrap();
        fs::write(dir.path().join("ws/code.rs"), "fn main() {}\nlet x = SECRET;\n").unwrap();
        let ws = crate::workspace_repo::insert_workspace(&db, "W", &dir.path().join("ws").to_string_lossy()).unwrap();
        let core = crate::CoreServices { db, pause: Default::default(), sensitive: crate::sensitive::SensitiveMatcher::new() };
        (core, dir, ws.id)
    }

    #[test]
    fn literal_search_finds_line_and_col() {
        let (core, _d, ws) = setup();
        let r = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, "SECRET", None, true, false, None)
        }).unwrap();
        assert_eq!(r.matches.len(), 1);
        assert_eq!(r.matches[0].path, "code.rs");
        assert_eq!(r.matches[0].line_no, 2);
        assert_eq!(r.matches[0].col, 9);
    }

    #[test]
    fn case_insensitive_when_requested() {
        let (core, _d, ws) = setup();
        let r = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, "secret", None, false, false, None)
        }).unwrap();
        assert_eq!(r.matches.len(), 1);
    }

    #[test]
    fn regex_mode() {
        let (core, _d, ws) = setup();
        let r = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, r"let \w+ =", None, true, true, None)
        }).unwrap();
        assert_eq!(r.matches.len(), 1);
    }

    #[test]
    fn invalid_regex_is_invalid_input_error() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, "(unclosed", None, true, true, None)
        }).unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::InvalidPatch); // reuse INVALID_* family? NO — see note
    }
}
```

**Note on invalid regex:** spec §6 has no `INVALID_ARGUMENT` code. Add one: extend `ErrorCode` with `InvalidArgument` → `"INVALID_ARGUMENT"` (update Task 2's test and the spec's table in the same commit — a spec amendment noted in the commit message). Use `INVALID_ARGUMENT` for bad regex, >8 read_files, and similar client-input problems; keep `RESULT_TOO_LARGE` only for bound-exceeded results. Adjust the `read_files_caps_at_8` test above to expect `ResultTooLarge` OR `InvalidArgument` — final decision: **>8 files = `INVALID_ARGUMENT`** (client sent too many), bounds on results stay `RESULT_TOO_LARGE`. Update both tests accordingly.

- [ ] **Step 2: Run to verify fail** — `cargo test -p core`.

- [ ] **Step 3: Implement** — shared walk helper in `ops/mod.rs`:

```rust
pub mod find_files;
pub mod list_dir;
pub mod read_file;
pub mod search_text;

use crate::error::{CoreError, ErrorCode};
use crate::CoreServices;
use sha2::{Digest, Sha256};
use std::path::Path;

pub const MAX_READ_BYTES: u64 = 262_144;
pub const MAX_LIST_ENTRIES: usize = 5_000;
pub const MAX_FIND_PATHS: usize = 500;
pub const MAX_SEARCH_MATCHES: usize = 200;
pub const MAX_READ_FILES: usize = 8;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex(&h.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Confinement + sensitive check shared by every op.
/// Returns the resolved absolute path and the path relative to root.
pub fn vet_path(
    core: &CoreServices,
    root: &Path,
    rel: &str,
) -> Result<(std::path::PathBuf, std::path::PathBuf), CoreError> {
    let target = root.join(rel);
    let resolved = crate::confinement::ensure_inside(root, &target)?;
    let rel_path = resolved
        .strip_prefix(dunce::canonicalize(root).map_err(CoreError::io)?)
        .map_err(|_| CoreError::new(ErrorCode::PathOutsideWorkspace, "resolved path lost its root"))?
        .to_path_buf();
    if core.sensitive.is_blocked(&rel_path) {
        return Err(CoreError::new(
            ErrorCode::SensitiveFileBlocked,
            format!("{rel} matches the sensitive-file deny list"),
        ));
    }
    Ok((resolved, rel_path))
}

/// Walker that respects .gitignore and skips node_modules.
pub fn walker(root: &Path) -> ignore::WalkBuilder {
    let mut b = ignore::WalkBuilder::new(root);
    b.hidden(false).git_ignore(true);
    b
}
```

`ops/list_dir.rs`:
```rust
use super::*;
use crate::error::{CoreError, ErrorCode};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
pub struct DirEntry {
    pub name: String,
    pub kind: &'static str,
    pub size: u64,
    pub modified: Option<String>,
}

#[derive(Serialize)]
pub struct ListDirResult {
    pub entries: Vec<DirEntry>,
    pub truncated: bool,
}

pub fn list_dir(core: &CoreServices, root: &Path, rel: &str, recursive: bool) -> Result<ListDirResult, CoreError> {
    let (resolved, _) = vet_path(core, root, rel)?;
    if !resolved.is_dir() {
        return Err(CoreError::new(ErrorCode::FileNotFound, format!("{rel} is not a directory")));
    }
    let mut entries = Vec::new();
    let mut truncated = false;
    let iter = if recursive {
        walker(&resolved).build().filter_map(|e| e.ok()).skip(1).collect::<Vec<_>>()
    } else {
        std::fs::read_dir(&resolved).map_err(CoreError::io)?
            .filter_map(|e| e.ok())
            .map(|e| ignore::DirEntry::from(e)) // see note
            .collect::<Vec<_>>()
    };
    ...
}
```

**Note:** `ignore::DirEntry::from(std::fs::DirEntry)` does not exist. For the non-recursive case, read entries directly with `std::fs::read_dir` and build `DirEntry` from `std::fs::Metadata`; only the recursive case uses the `ignore` walker. Final shape:

```rust
pub fn list_dir(core: &CoreServices, root: &Path, rel: &str, recursive: bool) -> Result<ListDirResult, CoreError> {
    let (resolved, rel_base) = vet_path(core, root, rel)?;
    if !resolved.is_dir() {
        return Err(CoreError::new(ErrorCode::FileNotFound, format!("{rel} is not a directory")));
    }
    let mut entries: Vec<DirEntry> = Vec::new();
    let mut truncated = false;

    let push = |entries: &mut Vec<DirEntry>, path: &Path, truncated: &mut bool| {
        if entries.len() >= MAX_LIST_ENTRIES {
            *truncated = true;
            return;
        }
        // hide sensitive entries from listings (default-deny: don't leak names)
        let rel_to_root = match path.strip_prefix(dunce::canonicalize(root).unwrap()) {
            Ok(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => return,
        };
        if core.sensitive.is_blocked(&rel_to_root) {
            return;
        }
        let md = std::fs::symlink_metadata(path).ok();
        let name = rel_to_root.to_string_lossy().replace('\\', "/");
        entries.push(DirEntry {
            name,
            kind: if path.is_dir() { "dir" } else { "file" },
            size: md.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: md.and_then(|m| m.modified().ok()).map(|t| {
                let t: time::SystemTime = t.into();
                t.format(&time::format_description::well_known::Rfc3339).unwrap_or_default()
            }),
        });
    };

    if recursive {
        for entry in walker(&resolved).build().flatten().skip(1) {
            push(&mut entries, entry.path(), &mut truncated);
            if truncated { break; }
        }
    } else {
        for entry in std::fs::read_dir(&resolved).map_err(CoreError::io)?.flatten() {
            push(&mut entries, &entry.path(), &mut truncated);
            if truncated { break; }
        }
    }
    let _ = rel_base;
    Ok(ListDirResult { entries, truncated })
}
```

`ops/find_files.rs`:
```rust
use super::*;
use crate::error::{CoreError, ErrorCode};
use globset::{Glob, GlobSetBuilder};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
pub struct FindResult {
    pub paths: Vec<String>,
    pub truncated: bool,
}

pub fn find_files(core: &CoreServices, root: &Path, pattern: &str) -> Result<FindResult, CoreError> {
    let glob = Glob::new(pattern)
        .map_err(|e| CoreError::new(ErrorCode::InvalidArgument, format!("bad glob: {e}")))?
        .literal_separator(true)
        .build();
    let mut builder = GlobSetBuilder::new();
    builder.add(glob);
    let set = builder.build().map_err(|e| CoreError::new(ErrorCode::InvalidArgument, e.to_string()))?;
    let root_c = dunce::canonicalize(root).map_err(CoreError::io)?;

    let mut paths = Vec::new();
    let mut truncated = false;
    for entry in walker(root).build().flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let rel = match entry.path().strip_prefix(&root_c) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if core.sensitive.is_blocked(rel) {
            continue;
        }
        if set.is_match(rel) {
            if paths.len() >= MAX_FIND_PATHS {
                truncated = true;
                break;
            }
            paths.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(FindResult { paths, truncated })
}
```

`ops/search_text.rs`:
```rust
use super::*;
use crate::error::{CoreError, ErrorCode};
use serde::Serialize;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Serialize)]
pub struct TextMatch {
    pub path: String,
    pub line_no: u64,
    pub line: String,
    pub col: u64,
}

#[derive(Serialize)]
pub struct SearchResult {
    pub matches: Vec<TextMatch>,
    pub truncated: bool,
}

pub fn search_text(
    core: &CoreServices, root: &Path, query: &str, path_prefix: Option<&str>,
    case_sensitive: bool, is_regex: bool, max_results: Option<usize>,
) -> Result<SearchResult, CoreError> {
    let cap = max_results.unwrap_or(MAX_SEARCH_MATCHES).min(MAX_SEARCH_MATCHES);
    let matcher: regex::bytes::Regex = {
        let mut pat = query.to_string();
        if !is_regex {
            pat = regex::escape(query);
        }
        let mut b = regex::bytes::RegexBuilder::new(&pat);
        b.case_insensitive(!case_sensitive);
        b.build().map_err(|e| CoreError::new(ErrorCode::InvalidArgument, format!("bad query: {e}")))?
    };
    let root_c = dunce::canonicalize(root).map_err(CoreError::io)?;
    let prefix_filter = path_prefix.map(|p| root_c.join(p));

    let mut matches = Vec::new();
    let mut truncated = false;
    'outer: for entry in walker(root).build().flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let rel = match entry.path().strip_prefix(&root_c) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if core.sensitive.is_blocked(rel) {
            continue;
        }
        if let Some(pf) = &prefix_filter {
            if !entry.path().starts_with(pf) {
                continue;
            }
        }
        let file = match std::fs::File::open(entry.path()) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        for (i, line) in BufReader::new(file).lines().enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break, // non-UTF8 or IO problem: skip rest of file
            };
            if let Some(m) = matcher.find(line.as_bytes()) {
                if matches.len() >= cap {
                    truncated = true;
                    break 'outer;
                }
                matches.push(TextMatch {
                    path: rel_str.clone(),
                    line_no: (i + 1) as u64,
                    line: line.chars().take(500).collect(),
                    col: line[..m.start()].chars().count() as u64 + 1,
                });
            }
        }
    }
    Ok(SearchResult { matches, truncated })
}
```

`ops/read_file.rs`:
```rust
use super::*;
use crate::error::{CoreError, ErrorCode};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize, Clone)]
pub struct ReadResult {
    pub content: String,
    pub revision: String,
    pub total_lines: u64,
    pub truncated: bool,
}

#[derive(Serialize)]
pub struct FileResult {
    pub path: String,
    pub result: Result<ReadResult, String>, // serialized CoreError {code,message}
}

pub fn read_file(core: &CoreServices, root: &Path, rel: &str,
                 offset_line: Option<u64>, max_lines: Option<u64>) -> Result<ReadResult, CoreError> {
    let (resolved, _) = vet_path(core, root, rel)?;
    let meta = std::fs::metadata(&resolved).map_err(|_| {
        CoreError::new(ErrorCode::FileNotFound, format!("{rel} not found"))
    })?;
    if meta.is_dir() {
        return Err(CoreError::new(ErrorCode::InvalidArgument, format!("{rel} is a directory")));
    }
    let bytes = std::fs::read(&resolved).map_err(CoreError::io)?;
    let revision = sha256_hex(&bytes);
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let total = lines.len() as u64;

    let start = offset_line.unwrap_or(0) as usize;
    if start > lines.len() {
        return Ok(ReadResult { content: String::new(), revision, total_lines: total, truncated: false });
    }
    let window: Vec<&str> = lines[start..]
        .iter()
        .copied()
        .take(max_lines.unwrap_or(u64::MAX) as usize)
        .collect();

    // byte bound: cut at MAX_READ_BYTES on a line boundary
    let mut content = window.concat();
    let mut truncated = start + window.len() < lines.len();
    if content.len() > MAX_READ_BYTES as usize {
        let mut cut = MAX_READ_BYTES as usize;
        while cut > 0 && !content.is_char_boundary(cut) {
            cut -= 1;
        }
        content.truncate(cut);
        truncated = true;
    }
    Ok(ReadResult { content, revision, total_lines: total, truncated })
}

pub fn read_files(core: &CoreServices, root: &Path, rels: &[String]) -> Result<Vec<FileResult>, CoreError> {
    if rels.len() > MAX_READ_FILES {
        return Err(CoreError::new(ErrorCode::InvalidArgument,
            format!("read_files accepts at most {MAX_READ_FILES} paths per call")));
    }
    Ok(rels.iter().map(|rel| {
        let result = read_file(core, root, rel, None, None)
            .map_err(|e| serde_json::json!({"code": e.code.as_str(), "message": e.message}).to_string());
        FileResult { path: rel.clone(), result }
    }).collect())
}
```

`lib.rs` gains `pub mod ops; pub mod pause; pub mod pipeline; pub mod confinement; pub mod sensitive;` (as introduced by their tasks). Also extend `ErrorCode` with `InvalidArgument` per the note above (error.rs match arm `"INVALID_ARGUMENT"`, Task 2 test updated).

- [ ] **Step 4: Run to verify pass** — `cargo test -p core` → all PASS.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "feat: read/search/list ops with confinement, sensitive filtering, revisions"
```

---

### Task 9: Write ops — create_file and apply_patch (atomic, revision-checked)

**Files:**
- Create: `crates/core/src/ops/write_file.rs`
- Modify: `crates/core/src/lib.rs`, `crates/core/src/ops/mod.rs`

**Interfaces:**
- Produces:
  - `pub fn atomic_write(target: &Path, bytes: &[u8]) -> std::io::Result<()>` — temp file `<name>.<uuid>.tmp` in same dir, write+flush, rename over target.
  - `ops::create_file(core, ws_root, rel: &str, content: &str) -> Result<CreateResult, CoreError>`; `CreateResult { path: String, revision: String }`; existing target → `FILE_EXISTS`; >10 MB → `INVALID_ARGUMENT`.
  - `ops::apply_patch(core, ws_root, args: &PatchArgs) -> Result<PatchResult, CoreError>`; `PatchArgs { rel: String, old_string: String, new_string: String, base_revision: String }`; `PatchResult { path: String, revision: String, bytes_changed: i64 }`.
  - apply_patch order: confinement → sensitive → read → `FILE_NOT_FOUND`/utf8 check → revision compare (`FILE_CHANGED_SINCE_READ`, detail has `expected`/`actual`) → anchor count (0 → `INVALID_PATCH` "anchor not found"; ≥2 → `INVALID_PATCH` "anchor matches N times; provide more context") → replace → atomic write → return new revision.

- [ ] **Step 1: Write failing tests** (inline)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::ops::read_file::read_file;
    use crate::pipeline::run_tool_audited;
    use std::fs;

    fn setup() -> (crate::CoreServices, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Database::open(&dir.path().join("t.db")).unwrap();
        fs::create_dir_all(dir.path().join("ws")).unwrap();
        let ws = crate::workspace_repo::insert_workspace(&db, "W", &dir.path().join("ws").to_string_lossy()).unwrap();
        let core = crate::CoreServices { db, pause: Default::default(), sensitive: crate::sensitive::SensitiveMatcher::new() };
        (core, dir, ws.id)
    }

    #[test]
    fn create_then_read_back() {
        let (core, _d, ws) = setup();
        let r = run_tool_audited(&core, "create_file", &ws, true, |_, root| {
            create_file(&core, root, "new.txt", "hello")
        }).unwrap();
        assert_eq!(r.revision.len(), 64);
        let on_disk = fs::read_to_string(_d.path().join("ws/new.txt")).unwrap();
        assert_eq!(on_disk, "hello");
    }

    #[test]
    fn create_existing_is_file_exists() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/x.txt"), "old").unwrap();
        let e = run_tool_audited(&core, "create_file", &ws, true, |_, root| {
            create_file(&core, root, "x.txt", "new")
        }).unwrap_err();
        assert_eq!(e.code, ErrorCode::FileExists);
        // original untouched (atomicity)
        assert_eq!(fs::read_to_string(d.path().join("ws/x.txt")).unwrap(), "old");
    }

    #[test]
    fn create_too_large_rejected() {
        let (core, _d, ws) = setup();
        let big = "x".repeat(10 * 1024 * 1024 + 1);
        let e = run_tool_audited(&core, "create_file", &ws, true, |_, root| {
            create_file(&core, root, "big.txt", &big)
        }).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn create_into_sensitive_path_blocked() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "create_file", &ws, true, |_, root| {
            create_file(&core, root, ".env", "SECRET=1")
        }).unwrap_err();
        assert_eq!(e.code, ErrorCode::SensitiveFileBlocked);
    }

    #[test]
    fn patch_replaces_unique_anchor() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/f.txt"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
        let rev0 = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", None, None)
        }).unwrap().revision;
        let r = run_tool_audited(&core, "apply_patch", &ws, true, |_, root| {
            apply_patch(&core, root, &PatchArgs {
                rel: "f.txt".into(),
                old_string: "println!(\"hi\");".into(),
                new_string: "println!(\"hello\");".into(),
                base_revision: rev0,
            })
        }).unwrap();
        assert_ne!(r.revision.len(), 0);
        assert_eq!(r.bytes_changed, 2); // "hi" -> "hello": +2 bytes... verify by content:
        let now = fs::read_to_string(d.path().join("ws/f.txt")).unwrap();
        assert_eq!(now, "fn main() {\n    println!(\"hello\");\n}\n");
    }

    #[test]
    fn patch_ambiguous_anchor_rejected() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/f.txt"), "a\na\n").unwrap();
        let rev = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", None, None)
        }).unwrap().revision;
        let e = run_tool_audited(&core, "apply_patch", &ws, true, |_, root| {
            apply_patch(&core, root, &PatchArgs {
                rel: "f.txt".into(), old_string: "a".into(), new_string: "b".into(), base_revision: rev,
            })
        }).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidPatch);
    }

    #[test]
    fn patch_stale_revision_rejected() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/f.txt"), "v1").unwrap();
        let stale_rev = crate::ops::sha256_hex(b"v0");
        let e = run_tool_audited(&core, "apply_patch", &ws, true, |_, root| {
            apply_patch(&core, root, &PatchArgs {
                rel: "f.txt".into(), old_string: "v1".into(), new_string: "v2".into(), base_revision: stale_rev,
            })
        }).unwrap_err();
        assert_eq!(e.code, ErrorCode::FileChangedSinceRead);
        assert_eq!(fs::read_to_string(d.path().join("ws/f.txt")).unwrap(), "v1");
    }

    #[test]
    fn patch_crlf_file_preserves_line_endings() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/f.txt"), "one\r\ntwo\r\n").unwrap();
        let rev = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", None, None)
        }).unwrap().revision;
        run_tool_audited(&core, "apply_patch", &ws, true, |_, root| {
            apply_patch(&core, root, &PatchArgs {
                rel: "f.txt".into(), old_string: "two".into(), new_string: "TWO".into(), base_revision: rev,
            })
        }).unwrap();
        assert_eq!(fs::read_to_string(d.path().join("ws/f.txt")).unwrap(), "one\r\nTWO\r\n");
    }
}
```

- [ ] **Step 2: Run to verify fail.**

- [ ] **Step 3: Implement**

`ops/write_file.rs`:
```rust
use super::*;
use crate::error::{CoreError, ErrorCode};
use serde::Serialize;
use std::path::Path;

pub const MAX_CREATE_BYTES: usize = 10 * 1024 * 1024;

pub fn atomic_write(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = target.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "target has no parent dir")
    })?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    match std::fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[derive(Serialize)]
pub struct CreateResult {
    pub path: String,
    pub revision: String,
}

pub fn create_file(core: &CoreServices, root: &Path, rel: &str, content: &str) -> Result<CreateResult, CoreError> {
    let (resolved, rel_path) = vet_path(core, root, rel)?;
    if content.len() > MAX_CREATE_BYTES {
        return Err(CoreError::new(ErrorCode::InvalidArgument,
            format!("content exceeds {MAX_CREATE_BYTES} bytes")));
    }
    if resolved.exists() {
        return Err(CoreError::new(ErrorCode::FileExists, format!("{rel} already exists")));
    }
    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent).map_err(CoreError::io)?;
    }
    atomic_write(&resolved, content.as_bytes()).map_err(CoreError::io)?;
    Ok(CreateResult {
        path: rel_path.to_string_lossy().replace('\\', "/"),
        revision: crate::ops::sha256_hex(content.as_bytes()),
    })
}

#[derive(Serialize)]
pub struct PatchArgs {
    pub rel: String,
    pub old_string: String,
    pub new_string: String,
    pub base_revision: String,
}

#[derive(Serialize)]
pub struct PatchResult {
    pub path: String,
    pub revision: String,
    pub bytes_changed: i64,
}

pub fn apply_patch(core: &CoreServices, root: &Path, args: &PatchArgs) -> Result<PatchResult, CoreError> {
    let (resolved, rel_path) = vet_path(core, root, &args.rel)?;
    let bytes = std::fs::read(&resolved).map_err(|_| {
        CoreError::new(ErrorCode::FileNotFound, format!("{} not found", args.rel))
    })?;
    let text = String::from_utf8(bytes.clone()).map_err(|_| {
        CoreError::new(ErrorCode::InvalidArgument, format!("{} is not valid UTF-8", args.rel))
    })?;

    let actual_rev = crate::ops::sha256_hex(&bytes);
    if actual_rev != args.base_revision {
        return Err(CoreError::new(ErrorCode::FileChangedSinceRead,
            "file changed since it was read; re-read and retry")
            .with_detail(serde_json::json!({
                "expected": args.base_revision, "actual": actual_rev })));
    }

    let count = text.matches(&args.old_string).count();
    match count {
        0 => return Err(CoreError::new(ErrorCode::InvalidPatch,
            "anchor not found in file; re-read the file first")),
        n if n > 1 => return Err(CoreError::new(ErrorCode::InvalidPatch,
            format!("anchor matches {n} times; include more surrounding context to make it unique"))),
        _ => {}
    }

    let patched = text.replacen(&args.old_string, &args.new_string, 1);
    atomic_write(&resolved, patched.as_bytes()).map_err(CoreError::io)?;
    Ok(PatchResult {
        path: rel_path.to_string_lossy().replace('\\', "/"),
        revision: crate::ops::sha256_hex(patched.as_bytes()),
        bytes_changed: patched.len() as i64 - text.len() as i64,
    })
}
```

`ops/mod.rs` gains `pub mod write_file;`. Note `String::matches`/`replacen` operate on `char` boundaries and preserve CRLF untouched — the CRLF test passes naturally.

- [ ] **Step 4: Run to verify pass** — `cargo test -p core` → all PASS.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "feat: atomic create_file and revision-checked apply_patch"
```

---

### Task 10: MCP server — real tool surface over the pipeline

**Files:**
- Modify: `src-tauri/src/server.rs` (replace EchoServer with `WorkspaceServer`)

**Interfaces:**
- Consumes: `core::pipeline::run_tool_audited`, all `core::ops::*` functions, `CoreServices`.
- Produces: `pub async fn serve(core: Arc<CoreServices>) -> std::io::Result<u16>` (signature unchanged from Task 1). 7 tools registered with names exactly: `list_directory`, `find_files`, `search_text`, `read_file`, `read_files`, `create_file`, `apply_patch`. Read tools annotated `read_only_hint = true`; write tools `read_only_hint = false`. Errors serialized as `{"code": "...", "message": "...", "detail": ...}` inside an `isError` result.

- [ ] **Step 1: Implement the tool server** (integration-tested via the rmcp client in Step 2)

```rust
use std::sync::Arc;
use axum::Router;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt, tool, tool_handler, tool_router,
    handler::server::router::tool::ToolRouter,
    model::{AnnotateAble, CallToolResult, Content, ProtocolVersion, ServerCapabilities, ServerInfo},
    transport::streamable_http_server::{
        StreamableHttpService, session::local::LocalSessionManager,
    },
};
use core::error::CoreError;
use core::CoreServices;

fn err_result(e: &CoreError) -> CallToolResult {
    let payload = serde_json::json!({
        "code": e.code.as_str(),
        "message": e.message,
        "detail": e.detail,
    });
    CallToolResult::error(vec![Content::text(payload.to_string())])
}

fn ok_json<T: serde::Serialize>(v: &T) -> CallToolResult {
    CallToolResult::success(vec![
        Content::text(serde_json::to_string_pretty(v).unwrap_or_default()),
    ])
}

#[derive(Clone)]
pub struct WorkspaceServer {
    core: Arc<CoreServices>,
    tool_router: ToolRouter<Self>,
}

// ---- argument structs (all take workspace_id first) ----

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ListDirArgs {
    pub workspace_id: String,
    /// Path relative to the workspace root, e.g. "." or "src"
    #[serde(default = "default_dot")]
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}
fn default_dot() -> String { ".".into() }

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct FindArgs {
    pub workspace_id: String,
    /// Glob pattern relative to workspace root, e.g. "**/*.rs"
    pub pattern: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    pub workspace_id: String,
    pub query: String,
    /// Restrict search to this subtree
    #[serde(default)]
    pub path_prefix: Option<String>,
    #[serde(default)]
    pub case_sensitive: bool,
    /// Treat query as a regular expression (default: literal text)
    #[serde(default)]
    pub is_regex: bool,
    #[serde(default)]
    pub max_results: Option<usize>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ReadArgs {
    pub workspace_id: String,
    pub path: String,
    #[serde(default)]
    pub offset_line: Option<u64>,
    #[serde(default)]
    pub max_lines: Option<u64>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ReadManyArgs {
    pub workspace_id: String,
    /// Up to 8 paths relative to the workspace root
    pub paths: Vec<String>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct CreateArgs {
    pub workspace_id: String,
    pub path: String,
    pub content: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct PatchArgs {
    pub workspace_id: String,
    pub path: String,
    /// Exact text to replace; must occur exactly once in the current file
    pub old_string: String,
    pub new_string: String,
    /// sha256 revision from the last read_file of this path
    pub base_revision: String,
}

#[tool_router]
impl WorkspaceServer {
    pub fn new(core: Arc<CoreServices>) -> Self {
        Self { core, tool_router: Self::tool_router() }
    }

    #[tool(description = "List entries of a directory inside an approved workspace",
           annotation(read_only_hint = true))]
    fn list_directory(&self, #[tool(aggr)] ListDirArgs { workspace_id, path, recursive }: ListDirArgs) -> CallToolResult {
        match core::pipeline::run_tool_audited(&self.core, "list_directory", &workspace_id, false, |_, root| {
            core::ops::list_dir::list_dir(&self.core, root, &path, recursive)
        }) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(description = "Find files by glob pattern inside an approved workspace",
           annotation(read_only_hint = true))]
    fn find_files(&self, #[tool(aggr)] FindArgs { workspace_id, pattern }: FindArgs) -> CallToolResult {
        match core::pipeline::run_tool_audited(&self.core, "find_files", &workspace_id, false, |_, root| {
            core::ops::find_files::find_files(&self.core, root, &pattern)
        }) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(description = "Search text (literal or regex) across files in an approved workspace",
           annotation(read_only_hint = true))]
    fn search_text(&self, #[tool(aggr)] a: SearchArgs) -> CallToolResult {
        match core::pipeline::run_tool_audited(&self.core, "search_text", &a.workspace_id, false, |_, root| {
            core::ops::search_text::search_text(
                &self.core, root, &a.query, a.path_prefix.as_deref(),
                a.case_sensitive, a.is_regex, a.max_results)
        }) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(description = "Read a file (with line offset/limit). Returns sha256 revision for patching.",
           annotation(read_only_hint = true))]
    fn read_file(&self, #[tool(aggr)] a: ReadArgs) -> CallToolResult {
        match core::pipeline::run_tool_audited(&self.core, "read_file", &a.workspace_id, false, |_, root| {
            core::ops::read_file::read_file(&self.core, root, &a.path, a.offset_line, a.max_lines)
        }) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(description = "Read up to 8 files at once; per-file results",
           annotation(read_only_hint = true))]
    fn read_files(&self, #[tool(aggr)] a: ReadManyArgs) -> CallToolResult {
        match core::pipeline::run_tool_audited(&self.core, "read_files", &a.workspace_id, false, |_, root| {
            core::ops::read_file::read_files(&self.core, root, &a.paths)
        }) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(description = "Create a new file (fails if it exists)",
           annotation(read_only_hint = false))]
    fn create_file(&self, #[tool(aggr)] a: CreateArgs) -> CallToolResult {
        match core::pipeline::run_tool_audited(&self.core, "create_file", &a.workspace_id, true, |_, root| {
            core::ops::write_file::create_file(&self.core, root, &a.path, &a.content)
        }) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(description = "Replace one unique occurrence of old_string in a file. Requires base_revision from a prior read.",
           annotation(read_only_hint = false))]
    fn apply_patch(&self, #[tool(aggr)] a: PatchArgs) -> CallToolResult {
        match core::pipeline::run_tool_audited(&self.core, "apply_patch", &a.workspace_id, true, |_, root| {
            core::ops::write_file::apply_patch(&self.core, root, &core::ops::write_file::PatchArgs {
                rel: a.path, old_string: a.old_string, new_string: a.new_string, base_revision: a.base_revision,
            })
        }) {
            Ok(r) => ok_json(&r),
            Err(e) => err_result(&e),
        }
    }
}

#[tool_handler]
impl ServerHandler for WorkspaceServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: rmcp::model::Implementation::from_build_env(),
            instructions: Some(
                "File access scoped to user-approved workspace folders. \
                 Every tool takes workspace_id. Read tools return a `revision` \
                 (sha256) that apply_patch requires as base_revision.".into()),
        }
    }
}

pub async fn serve(core: Arc<CoreServices>) -> std::io::Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let service = StreamableHttpService::new(
        move || Ok(WorkspaceServer::new(core.clone())),
        LocalSessionManager::default().into(),
        Default::default(),
    );
    let app = Router::new().nest_service("/mcp", service);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("mcp server error: {e}");
        }
    });
    Ok(port)
}
```

If the `#[tool(... annotation(...) )]` attribute syntax differs in the pinned rmcp version, consult `docs.rs/rmcp` for the exact `annotations` helper on `#[tool]` and adapt — the tool names and behavior MUST stay identical.

- [ ] **Step 2: Write the integration test** — `src-tauri/tests/mcp_integration.rs`:

```rust
// In-process client -> real axum/rmcp stack against a temp workspace.
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::{StreamableHttpService, session::local::LocalSessionManager};
use axum::Router;

async fn spawn_server() -> (u16, core::CoreServices, tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let db = core::db::Database::open(&dir.path().join("t.db")).unwrap();
    std::fs::create_dir_all(dir.path().join("ws")).unwrap();
    std::fs::write(dir.path().join("ws/hello.txt"), "hello world\n").unwrap();
    let ws = core::workspace_repo::insert_workspace(&db, "W", &dir.path().join("ws").to_string_lossy()).unwrap();
    let core = core::CoreServices { db, pause: Default::default(), sensitive: core::sensitive::SensitiveMatcher::new() };
    let core = std::sync::Arc::new(core);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let service = StreamableHttpService::new(
        move || Ok(local_workspace_mcp_manager::server::WorkspaceServer::new(core.clone())),
        LocalSessionManager::default().into(),
        Default::default(),
    );
    let app = Router::new().nest_service("/mcp", service);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (port, /* core not returned: Arc moved */ todo!(), dir, ws.id)
}
```

**Restructure** so tests can reach the core (the Arc must be cloned before the move):

```rust
async fn spawn_server() -> (u16, std::sync::Arc<core::CoreServices>, tempfile::TempDir, String) {
    // ... build core as above ...
    let core = std::sync::Arc::new(core);
    let core_for_server = core.clone();
    // ... service closure uses core_for_server ...
    // return (port, core, dir, ws.id)
}
```

Test body (requires `src-tauri` to be a lib+bin crate so tests can import it: add `[lib] name = "local_workspace_mcp_manager"` with `src/lib.rs` holding `pub mod server;` and `main.rs` calling `local_workspace_mcp_manager::run()` — standard Tauri 2 layout; do this refactor as part of this task):

```rust
#[tokio::test]
async fn full_read_write_roundtrip_over_mcp() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    use rmcp::transport::StreamableHttpClientTransport;
    let client = ().serve(StreamableHttpClientTransport::from_uri(format!("http://127.0.0.1:{port}/mcp"))).await.unwrap();

    // list
    let res = client.call_tool(rmcp::model::CallToolRequestParam {
        name: "list_directory".into(), arguments: Some(serde_json::json!({
            "workspace_id": ws_id, "path": "." }).as_object().unwrap().clone()),
    }).await.unwrap();
    assert!(serde_json::to_string(&res).unwrap().contains("hello.txt"));

    // read + patch
    let res = client.call_tool(rmcp::model::CallToolRequestParam {
        name: "read_file".into(), arguments: Some(serde_json::json!({
            "workspace_id": ws_id, "path": "hello.txt" }).as_object().unwrap().clone()),
    }).await.unwrap();
    let text = serde_json::to_string(&res).unwrap();
    let rev: String = extract_json(&text)["revision"].as_str().unwrap().into();

    let res = client.call_tool(rmcp::model::CallToolRequestParam {
        name: "apply_patch".into(), arguments: Some(serde_json::json!({
            "workspace_id": ws_id, "path": "hello.txt",
            "old_string": "world", "new_string": "there", "base_revision": rev
            }).as_object().unwrap().clone()),
    }).await.unwrap();
    assert!(serde_json::to_string(&res).unwrap().contains("there"));
    assert_eq!(std::fs::read_to_string(_dir.path().join("ws/hello.txt")).unwrap(), "hello there\n");
    let _ = core; // audit rows visible via core.db if further asserts needed
}

fn extract_json(text: &str) -> serde_json::Value {
    // first '[' or '{' to last ']' or '}' — content may be wrapped in SSE frames in some versions
    let s = text.trim();
    let start = s.find(['{', '[']).unwrap();
    let end = s.rfind(['}', ']']).unwrap();
    serde_json::from_str(&s[start..=end]).unwrap()
}

#[tokio::test]
async fn pause_blocks_tool_calls() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    // ... client setup as above ...
    core.pause.set(true);
    let res = client.call_tool(rmcp::model::CallToolRequestParam {
        name: "list_directory".into(), arguments: Some(serde_json::json!({
            "workspace_id": ws_id }).as_object().unwrap().clone()),
    }).await.unwrap();
    assert!(serde_json::to_string(&res).unwrap().contains("ACCESS_PAUSED"));
}
```

Add to `src-tauri/Cargo.toml` `[dev-dependencies]`: `tempfile.workspace = true`, `rmcp` client feature (`rmcp = { version = "3.1", features = ["client", "transport-streamable-http-client"] }` — extend the existing rmcp dependency's features list).

- [ ] **Step 3: Run integration tests**

Run: `cargo test -p local-workspace-mcp-manager`
Expected: both tests PASS. Fix any rmcp API drift by consulting docs.rs for the pinned version — tool names/behavior are the contract, macro spelling may differ.

- [ ] **Step 4: Commit**

```powershell
git add -A
git commit -m "feat: full MCP tool surface (7 tools) with integration tests over real axum stack"
```

---

### Task 11: Tauri commands + app wiring

**Files:**
- Modify: `src-tauri/src/lib.rs`, `src-tauri/src/main.rs`
- Create: `src-tauri/src/commands.rs`, `src-tauri/src/settings.rs`

**Interfaces:**
- Consumes: `CoreServices`, workspace repo, audit, `PauseFlag`.
- Produces (Tauri commands, all `Result<T, String>` where `String` is the serialized error JSON):
  - `add_workspace(state, root_path: String, name: Option<String>) -> Workspace` — canonicalizes via `dunce::canonicalize` (must exist and be a dir), inserts with default caps, audits `source: "ui", op: "add_workspace"`.
  - `list_workspaces(state) -> Vec<Workspace>`
  - `set_capabilities(state, id: String, can_read: bool, can_write: bool) -> ()` — audits `op: "set_capabilities"`.
  - `remove_workspace(state, id: String) -> ()` — audits `op: "remove_workspace"` (files untouched).
  - `set_paused(state, paused: bool) -> ()` — sets flag, audits `op: "pause"/"resume"`, emits `"access-paused"` event.
  - `recent_audit(state) -> Vec<AuditEvent>` (100 newest).
  - `server_status(state) -> Status { port: u16, paused: bool }`.
  - Tauri-managed state: `struct AppState { core: Arc<CoreServices>, port: u16 }`.
  - `settings.rs`: `pub struct Settings { pub tunnel_id: String, pub tunnel_api_key: String }` with `load(app_data_dir)/save(app_data_dir)` to `settings.json` (serde_json, created on first save).

- [ ] **Step 1: Implement settings.rs and commands.rs** (per interfaces above; each command is 5–15 lines calling the core functions from Tasks 3–4 and auditing via `audit::record_simple(..., source: "ui", ...)`).

`src-tauri/src/lib.rs`:
```rust
mod commands;
pub mod server;
mod settings;

use std::sync::Arc;
use tauri::Manager;

pub struct AppState {
    pub core: Arc<CoreServices>,
    pub port: u16,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let core = Arc::new(core::CoreServices::open(&dir.join("manager.db")).map_err(|e| e.to_string())?);
            let core_for_server = core.clone();
            // block briefly to get the port before UI loads
            let port = tauri::async_runtime::block_on(server::serve(core_for_server))?;
            app.manage(AppState { core, port });
            println!("MCP server: http://127.0.0.1:{port}/mcp");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_workspace, commands::list_workspaces,
            commands::set_capabilities, commands::remove_workspace,
            commands::set_paused, commands::recent_audit, commands::server_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

`main.rs`:
```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() { local_workspace_mcp_manager::run() }
```

- [ ] **Step 2: Build and manual smoke**

Run: `cargo build` then `cargo tauri dev`.
Expected: window opens; devtools console shows no errors; MCP port printed to terminal. (UI is still the scaffold placeholder.)

- [ ] **Step 3: Commit**

```powershell
git add -A
git commit -m "feat: tauri commands for workspace management, pause, and audit"
```

---

### Task 12: tunnel-client sidecar + process lifecycle

**Files:**
- Create: `src-tauri/src/tunnel.rs`
- Modify: `src-tauri/tauri.conf.json`, `src-tauri/src/lib.rs`, `src-tauri/src/commands.rs` (add `configure_tunnel`, `tunnel_status` commands)

**Interfaces:**
- Produces:
  - `pub struct TunnelHandle(Mutex<Option<Child>>)` with `pub fn spawn(app_handle, settings, port) -> Result<(), String>` and `pub fn kill(&self)`.
  - `TunnelStatus` enum `Stopped | Connecting | Connected | Error(String)` pushed to the UI via Tauri event `"tunnel-status"`.
  - Commands: `configure_tunnel(state, tunnel_id, api_key) -> ()` (saves settings, spawns), `stop_tunnel(state) -> ()`, `tunnel_status(state) -> String`.
- Sidecar binary: `tunnel-client-x86_64-pc-windows-msvc.exe` from github.com/openai/tunnel-client releases v0.0.12, placed in `src-tauri/binaries/`, declared in `tauri.conf.json` under `"bundle": { "externalBin": ["binaries/tunnel-client"] }` (Tauri appends the target triple).

- [ ] **Step 1: Download the sidecar**

```powershell
mkdir src-tauri\binaries
# Download the Windows release binary from https://github.com/openai/tunnel-client/releases (v0.0.12)
# and save it as:
#   src-tauri\binaries\tunnel-client-x86_64-pc-windows-msvc.exe
```

Verify: `.\src-tauri\binaries\tunnel-client-x86_64-pc-windows-msvc.exe --help` — note the exact flag names for tunnel id, API key, and upstream URL; adjust `spawn` below if they differ from the assumed flags.

- [ ] **Step 2: Implement tunnel.rs**

```rust
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

pub struct TunnelHandle(pub Mutex<Option<Child>>);

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelStatus {
    Stopped,
    Connecting,
    Connected,
    Error(String),
}

impl TunnelStatus {
    fn emit(&self, app: &AppHandle) {
        let _ = app.emit("tunnel-status", self);
    }
}

pub fn spawn(app: &AppHandle, tunnel_id: &str, api_key: &str, port: u16) -> Result<(), String> {
    let exe = app.path().resource_dir()
        .map_err(|e| e.to_string())?
        .join("binaries/tunnel-client-x86_64-pc-windows-msvc.exe");
    let child = Command::new(&exe)
        .args([
            "--tunnel-id", tunnel_id,
            "--api-key", api_key,
            "--url", &format!("http://127.0.0.1:{port}/mcp"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn tunnel-client: {e}"))?;
    let state = app.state::<TunnelHandle>();
    *state.0.lock().unwrap() = Some(child);
    TunnelStatus::Connected.emit(app);
    Ok(())
}

pub fn kill(app: &AppHandle) {
    let state = app.state::<TunnelHandle>();
    if let Some(mut child) = state.0.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    TunnelStatus::Stopped.emit(app);
}
```

- [ ] **Step 3: Wire lifecycle in lib.rs**

In `setup`: `app.manage(TunnelHandle(Mutex::new(None)));` then load settings and, if `tunnel_id` non-empty, spawn. On exit:

```rust
.build(tauri::generate_context!())
.expect("build tauri app")
.run(|app_handle, event| {
    if let tauri::RunEvent::ExitRequested { .. } = event {
        tunnel::kill(app_handle);
    }
});
```

(Replaces `.run(...)` with `.build(...).run(closure)`.)

- [ ] **Step 4: Lifecycle test with a stub binary** — `src-tauri/tests/tunnel_lifecycle.rs`:

```rust
// Spawn a stub "tunnel-client" (a copy of cmd /c ping -n 30 127.0.0.1 style
// long-running process is NOT portable; use powershell -Command "Start-Sleep 30")
// and assert kill() terminates it.
#[test]
fn kill_terminates_child() {
    let mut child = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
        .spawn().unwrap();
    let pid = child.id();
    child.kill().unwrap();
    let _ = child.wait();
    // After wait, the process must be gone.
    assert!(!is_alive(pid));
}

fn is_alive(pid: u32) -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}")])
        .output().map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}
```

Run: `cargo test -p local-workspace-mcp-manager` → PASS.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "feat: tunnel-client sidecar with explicit lifecycle teardown"
```

---

### Task 13: React UI — single screen

**Files:**
- Modify: `src/App.tsx`
- Create: `src/api.ts`, `src/components/WorkspaceList.tsx`, `src/components/PauseButton.tsx`, `src/components/AuditTable.tsx`, `src/components/StatusBar.tsx`, `src/components/SettingsDialog.tsx`

**Interfaces:**
- Consumes: Tauri commands from Task 11 (`add_workspace`, `list_workspaces`, `set_capabilities`, `remove_workspace`, `set_paused`, `recent_audit`, `server_status`) and Task 12 (`configure_tunnel`, `stop_tunnel`, `tunnel_status`); events `"tunnel-status"`, `"access-paused"`. Dialog via `@tauri-apps/plugin-dialog` `open({ directory: true })`.

- [ ] **Step 1: Implement api.ts**

```ts
import { invoke } from "@tauri-apps/api/core";

export interface Workspace {
  id: string; name: string; root_path: string;
  can_read: boolean; can_write: boolean;
  created_at: string; updated_at: string;
}
export interface AuditEvent {
  id: number; ts: string; source: string;
  tool?: string | null; workspace_id?: string | null;
  op: string; decision: string; error_code?: string | null;
  path?: string | null; detail?: unknown;
}
export interface ServerStatus { port: number; paused: boolean }

export const api = {
  addWorkspace: (rootPath: string, name?: string) =>
    invoke<Workspace>("add_workspace", { rootPath, name: name ?? null }),
  listWorkspaces: () => invoke<Workspace[]>("list_workspaces"),
  setCapabilities: (id: string, canRead: boolean, canWrite: boolean) =>
    invoke<void>("set_capabilities", { id, canRead, canWrite }),
  removeWorkspace: (id: string) => invoke<void>("remove_workspace", { id }),
  setPaused: (paused: boolean) => invoke<void>("set_paused", { paused }),
  recentAudit: () => invoke<AuditEvent[]>("recent_audit"),
  serverStatus: () => invoke<ServerStatus>("server_status"),
  configureTunnel: (tunnelId: string, apiKey: string) =>
    invoke<void>("configure_tunnel", { tunnelId, apiKey }),
  stopTunnel: () => invoke<void>("stop_tunnel"),
};
```

- [ ] **Step 2: Implement App.tsx and components** — layout per spec §7 (connection status bar, workspace rows with read/write toggles + remove, add button opening the folder picker, big Pause button, audit table of 100). Full component code:

`src/App.tsx`:
```tsx
import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, AuditEvent, ServerStatus, Workspace } from "./api";
import StatusBar from "./components/StatusBar";
import WorkspaceList from "./components/WorkspaceList";
import PauseButton from "./components/PauseButton";
import AuditTable from "./components/AuditTable";
import SettingsDialog from "./components/SettingsDialog";

export default function App() {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [audit, setAudit] = useState<AuditEvent[]>([]);
  const [status, setStatus] = useState<ServerStatus | null>(null);
  const [paused, setPaused] = useState(false);
  const [tunnel, setTunnel] = useState<string>("stopped");
  const [showSettings, setShowSettings] = useState(false);

  const refresh = useCallback(async () => {
    setWorkspaces(await api.listWorkspaces());
    setAudit(await api.recentAudit());
  }, []);

  useEffect(() => {
    (async () => {
      const s = await api.serverStatus();
      setStatus(s);
      setPaused(s.paused);
      await refresh();
    })();
    const un1 = listen<string>("tunnel-status", (e) => setTunnel(JSON.stringify(e.payload).replace(/"/g, "")));
    const un2 = listen("access-paused", () => refresh());
    return () => { un1.then((f) => f()); un2.then((f) => f()); };
  }, [refresh]);

  const togglePause = async () => {
    const next = !paused;
    await api.setPaused(next);
    setPaused(next);
    await refresh();
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12, padding: 16, fontFamily: "system-ui" }}>
      <StatusBar port={status?.port} tunnel={tunnel} onOpenSettings={() => setShowSettings(true)} />
      <WorkspaceList workspaces={workspaces} onChanged={refresh} />
      <PauseButton paused={paused} onToggle={togglePause} />
      <AuditTable events={audit} />
      {showSettings && <SettingsDialog onClose={() => setShowSettings(false)} />}
    </div>
  );
}
```

`src/components/WorkspaceList.tsx`:
```tsx
import { open } from "@tauri-apps/plugin-dialog";
import { api, Workspace } from "../api";

interface Props { workspaces: Workspace[]; onChanged: () => void }

export default function WorkspaceList({ workspaces, onChanged }: Props) {
  const add = async () => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") {
      await api.addWorkspace(dir);
      onChanged();
    }
  };
  const toggle = async (ws: Workspace, cap: "read" | "write") => {
    if (cap === "read") await api.setCapabilities(ws.id, !ws.can_read, ws.can_write);
    else await api.setCapabilities(ws.id, ws.can_read, !ws.can_write);
    onChanged();
  };
  const remove = async (ws: Workspace) => {
    if (confirm(`Remove workspace "${ws.name}"? Files on disk are NOT deleted.`)) {
      await api.removeWorkspace(ws.id);
      onChanged();
    }
  };
  return (
    <section>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <h2 style={{ margin: "8px 0" }}>Workspaces</h2>
        <button onClick={add}>+ Add folder</button>
      </div>
      {workspaces.map((ws) => (
        <div key={ws.id} style={{ display: "flex", gap: 12, alignItems: "center", padding: 8, borderBottom: "1px solid #ddd" }}>
          <div style={{ flex: 1 }}>
            <strong>{ws.name}</strong>
            <div style={{ fontSize: 12, color: "#666" }}>{ws.root_path}</div>
          </div>
          <label><input type="checkbox" checked={ws.can_read} onChange={() => toggle(ws, "read")} /> Read</label>
          <label><input type="checkbox" checked={ws.can_write} onChange={() => toggle(ws, "write")} /> Write</label>
          <button onClick={() => remove(ws)}>Remove</button>
        </div>
      ))}
      {workspaces.length === 0 && <p style={{ color: "#888" }}>No workspaces yet — add a folder to let ChatGPT access it.</p>}
    </section>
  );
}
```

`src/components/PauseButton.tsx`:
```tsx
interface Props { paused: boolean; onToggle: () => void }
export default function PauseButton({ paused, onToggle }: Props) {
  return (
    <button onClick={onToggle}
      style={{ padding: "12px 24px", fontSize: 16, fontWeight: 700,
               background: paused ? "#c62828" : "#2e7d32", color: "white", border: "none", borderRadius: 6 }}>
      {paused ? "▶ Resume access" : "⏸ Pause all access"}
    </button>
  );
}
```

`src/components/StatusBar.tsx`:
```tsx
interface Props { port?: number; tunnel: string; onOpenSettings: () => void }
export default function StatusBar({ port, tunnel, onOpenSettings }: Props) {
  const color = tunnel === "connected" ? "green" : tunnel === "error" ? "red" : "gray";
  return (
    <div style={{ display: "flex", gap: 16, alignItems: "center", padding: 8, background: "#f5f5f5", borderRadius: 6 }}>
      <span>MCP: 127.0.0.1:{port ?? "…"}/mcp</span>
      <span style={{ color }}>Tunnel: {tunnel}</span>
      <button onClick={onOpenSettings} style={{ marginLeft: "auto" }}>Tunnel settings…</button>
    </div>
  );
}
```

`src/components/AuditTable.tsx`:
```tsx
import { AuditEvent } from "../api";
export default function AuditTable({ events }: { events: AuditEvent[] }) {
  return (
    <section>
      <h2 style={{ margin: "8px 0" }}>Audit log (latest 100)</h2>
      <div style={{ maxHeight: 240, overflowY: "auto" }}>
        <table style={{ width: "100%", fontSize: 12, borderCollapse: "collapse" }}>
          <thead><tr>
            <th align="left">Time</th><th align="left">Source</th><th align="left">Op</th>
            <th align="left">Decision</th><th align="left">Path</th>
          </tr></thead>
          <tbody>
            {events.map((e) => (
              <tr key={e.id}>
                <td>{e.ts}</td><td>{e.source}</td><td>{e.tool ?? e.op}</td>
                <td style={{ color: e.decision === "allow" ? "green" : e.decision === "deny" ? "red" : "orange" }}>
                  {e.decision}{e.error_code ? ` (${e.error_code})` : ""}
                </td>
                <td style={{ maxWidth: 300, overflow: "hidden", textOverflow: "ellipsis" }}>{e.path ?? ""}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
```

`src/components/SettingsDialog.tsx`:
```tsx
import { useState } from "react";
import { api } from "../api";

export default function SettingsDialog({ onClose }: { onClose: () => void }) {
  const [tunnelId, setTunnelId] = useState("");
  const [apiKey, setApiKey] = useState("");
  return (
    <div style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,.4)", display: "flex",
                  alignItems: "center", justifyContent: "center" }}>
      <div style={{ background: "white", padding: 24, borderRadius: 8, minWidth: 380, display: "flex",
                    flexDirection: "column", gap: 10 }}>
        <h3>Tunnel settings</h3>
        <label>Tunnel ID<input value={tunnelId} onChange={(e) => setTunnelId(e.target.value)} style={{ width: "100%" }} /></label>
        <label>API key<input type="password" value={apiKey} onChange={(e) => setApiKey(e.target.value)} style={{ width: "100%" }} /></label>
        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
          <button onClick={onClose}>Cancel</button>
          <button onClick={async () => { await api.configureTunnel(tunnelId, apiKey); onClose(); }}>Save & connect</button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Manual smoke via `cargo tauri dev`**

Checklist: add a folder → row appears; toggle write → persists after restart; Pause → turns red; audit table shows `add_workspace`/`pause` UI rows; settings dialog saves.

- [ ] **Step 4: Commit**

```powershell
git add -A
git commit -m "feat: single-screen UI with workspace list, pause, audit, tunnel settings"
```

---

### Task 14: End-to-end verification against real ChatGPT Web

**Files:**
- Modify: none (verification task); fix-forward changes if scenarios fail.

**Interfaces:**
- Consumes: everything. Requires the user's ChatGPT Plus account with Developer Mode, a tunnel endpoint created at platform.openai.com, and the tunnel id + API key.

- [ ] **Step 1: Build the release app**

Run: `cargo tauri build`
Expected: NSIS installer + runnable exe under `target/release/`. (First NSIS build downloads tooling; allow time.)

- [ ] **Step 2: Configure and connect**

Launch the app → Tunnel settings → enter tunnel id + API key → status shows `connected`.

- [ ] **Step 3: In ChatGPT Web, create the developer-mode app**

Settings → Apps & Connectors → Developer mode → create app, Connection type **Tunnel**, select the tunnel endpoint. Enable the app in a chat.

- [ ] **Step 4: Run acceptance scenarios A–H** (spec §10)

Work through each scenario conversationally and record pass/fail:
- **A**: list → read → patch → verify on disk.
- **B**: ask ChatGPT to read `../../../outside.txt` → expect `PATH_OUTSIDE_WORKSPACE` in the tool result; confirm an audit deny row in the UI.
- **C**: read `.env` → `SENSITIVE_FILE_BLOCKED`.
- **D**: hit Pause mid-conversation → next call `ACCESS_PAUSED`; resume works.
- **E**: edit a file externally after ChatGPT read it → patch → `FILE_CHANGED_SINCE_READ`.
- **F**: write toggle off → `apply_patch` → `WRITE_DISABLED`.
- **G**: quit app → Task Manager shows no orphan `tunnel-client` process → relaunch reconnects.
- **H**: two workspaces → cross-workspace path fails confinement.

- [ ] **Step 5: Fix-forward any failures, then commit**

```powershell
git add -A
git commit -m "test: E2E acceptance scenarios A-H verified against ChatGPT Web"
```

---

### Task 15: Coverage check + hardening sweep

**Files:**
- Modify: whatever the sweep finds (tests first).

**Interfaces:**
- Consumes: full test suite.
- Produces: coverage report ≥ 80% for `crates/core`; edge cases from spec §9 pinned as tests.

- [ ] **Step 1: Install and run coverage**

```powershell
cargo install cargo-llvm-cov
cargo llvm-cov --workspace --summary-only
```

Expected: `crates/core` ≥ 80% line coverage. Below target → add tests for uncovered branches in `confinement.rs`, `sensitive.rs`, `pipeline.rs`, `ops/*` first (these are the security-critical modules; UI glue is exempt).

- [ ] **Step 2: Add the spec §9 edge-case tests not yet covered**

Minimum additions (each a small `#[test]`, same style as prior tasks):
- `apply_patch` where `old_string == new_string` (no-op → allowed, revision unchanged).
- `read_file` on a file with CRLF only — `total_lines` counts `\r\n` correctly.
- `search_text` with `max_results` below the number of actual matches → `truncated: true`.
- `list_directory` on a file (not dir) → `FILE_NOT_FOUND` per Task 8's guard.
- Confinement: `root.join("sub/../../x")` rejected; `root.join("sub/../file.txt")` allowed.
- `create_file` into a non-existent nested dir (`a/b/c/new.txt`) → creates parents, succeeds.

- [ ] **Step 3: Run full suite + clippy**

```powershell
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

Expected: green.

- [ ] **Step 4: Commit**

```powershell
git add -A
git commit -m "test: hardening sweep, coverage >= 80% on core"
```

---

### Task 16: Packaging + README

**Files:**
- Create: `README.md`
- Modify: `src-tauri/tauri.conf.json` (final icons/metadata if needed)

**Interfaces:**
- Produces: distributable NSIS installer; README with setup steps (Rust, Node, tunnel endpoint creation, Developer Mode app config) and the security model summary.

- [ ] **Step 1: Write README.md** — sections: What it is; Prerequisites; Build (`npm install`, `cargo tauri build`); First-run setup (add folder, create tunnel endpoint at platform.openai.com, enter tunnel id + API key); Connect ChatGPT (Developer mode app, Tunnel connection); Security model (loopback-only, policy pipeline, sensitive deny-list, pause, append-only audit); Error codes table (copy from spec §6).

- [ ] **Step 2: Final release build + install smoke**

Run: `cargo tauri build` → install the NSIS output on this machine → run through scenario A once from the installed app.

- [ ] **Step 3: Commit**

```powershell
git add -A
git commit -m "chore: readme and release packaging"
```

---

## Task dependency notes

- Tasks 1–10 are strictly sequential (each consumes the previous task's interfaces).
- Task 11 depends on 10; Task 12 depends on 11 (uses `AppState.port`); Task 13 depends on 11+12 (commands + events).
- Task 14 (E2E) needs 12+13 done and the user's participation (ChatGPT account, tunnel endpoint).
- Tasks 15–16 can run after 14 or in parallel with it if E2E is blocked on user availability — but do NOT mark the milestone complete without 14.

## Spec coverage map

| Spec section | Task |
|---|---|
| §2 Architecture (loopback, sidecar, single process) | 1, 10, 12 |
| §3.1 workspaces table | 3 |
| §3.2 capability profile constants | 6, 8 (bounds), 9 (write limit) |
| §3.3 session_grants | 3 |
| §3.4 audit_events + append-only | 3 (schema/triggers), 4 (API), 7 (integration) |
| §4.1 pipeline + kill switch | 7 |
| §4.2 path confinement | 5 |
| §4.3 sensitive files | 6 |
| §4.4 atomic writes + revisions | 9 |
| §4.5 patch-first editing | 9 |
| §4.6 network posture | 1, 10 |
| §5 tool surface (7 tools) | 10 |
| §6 error model | 2 (+ INVALID_ARGUMENT amendment) |
| §7 UI | 13 |
| §8 lifecycle | 11, 12 |
| §9 testing | 5–10, 12, 15 |
| §10 scenarios A–H | 14 |
| §11 phases | mapped: P0→1, P1→3–7, P2→8, P3→9, P4→10–14, P5→15–16 |
