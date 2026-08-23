//! SQLite persistence: connection management, WAL setup, and schema migrations.

use crate::error::CoreError;
use std::path::Path;
use std::sync::Mutex;

/// Handle to the shared SQLite database.
///
/// The single connection is guarded by a `Mutex` because both the MCP server
/// and Tauri command handlers share one `Database` instance across threads.
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
    /// Opens (or creates) the database at `path`, enabling WAL mode and
    /// foreign keys, and applying idempotent migrations including the
    /// append-only triggers on `audit_events`.
    pub fn open(path: &Path) -> Result<Self, CoreError> {
        let conn = rusqlite::Connection::open(path).map_err(CoreError::from)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(CoreError::from)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(CoreError::from)?;
        conn.execute_batch(MIGRATIONS).map_err(CoreError::from)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Runs `f` with exclusive access to the underlying connection.
    pub fn with_conn<T>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>,
    ) -> Result<T, CoreError> {
        let conn = self.conn.lock().unwrap();
        f(&conn).map_err(CoreError::from)
    }
}
