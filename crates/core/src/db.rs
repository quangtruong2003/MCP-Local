//! SQLite persistence: connection management, WAL setup, and schema migrations.

use crate::{error::CoreError, models::AuditEvent};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub type AuditListener = Arc<dyn Fn(&AuditEvent) + Send + Sync>;

/// Handle to the shared SQLite database.
///
/// The single connection is guarded by a `Mutex` because both the MCP server
/// and Tauri command handlers share one `Database` instance across threads.
pub struct Database {
    conn: Mutex<rusqlite::Connection>,
    audit_listener: Mutex<Option<AuditListener>>,
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

/// Adds post-v1 columns to `workspaces` only when missing (SQLite has no
/// `ADD COLUMN IF NOT EXISTS`). Idempotent: re-opening a migrated DB is a no-op.
fn migrate_add_columns(conn: &rusqlite::Connection) -> Result<(), CoreError> {
    let existing: std::collections::HashSet<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(workspaces)")?;
        let names = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        names.into_iter().collect()
    };
    if !existing.contains("can_execute") {
        conn.execute_batch(
            "ALTER TABLE workspaces ADD COLUMN can_execute INTEGER NOT NULL DEFAULT 0;",
        )
        .map_err(CoreError::from)?;
    }
    if !existing.contains("execution_mode") {
        conn.execute_batch(
            "ALTER TABLE workspaces ADD COLUMN execution_mode TEXT NOT NULL DEFAULT 'approved_commands';",
        )
        .map_err(CoreError::from)?;
    }
    Ok(())
}

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
        migrate_add_columns(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            audit_listener: Mutex::new(None),
        })
    }

    pub fn set_audit_listener(&self, listener: AuditListener) {
        *self.audit_listener.lock().unwrap() = Some(listener);
    }

    pub(crate) fn notify_audit_listener(&self, event: &AuditEvent) {
        let listener = self.audit_listener.lock().unwrap().clone();
        if let Some(listener) = listener {
            listener(event);
        }
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
