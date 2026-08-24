//! Repository functions for workspaces and their permanent session grants.

use crate::db::Database;
use crate::error::{CoreError, ErrorCode};
use crate::models::{ExecutionMode, Workspace};
use std::path::PathBuf;

fn now_iso() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn row_to_ws(row: &rusqlite::Row) -> rusqlite::Result<Workspace> {
    let mode_raw: String = row.get("execution_mode")?;
    let execution_mode = ExecutionMode::from_storage(&mode_raw).map_err(|bad| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("corrupt execution_mode value {bad:?}: unknown execution mode").into(),
        )
    })?;
    Ok(Workspace {
        id: row.get("id")?,
        name: row.get("name")?,
        root_path: PathBuf::from(row.get::<_, String>("root_path")?),
        can_read: row.get::<_, i64>("can_read")? != 0,
        can_write: row.get::<_, i64>("can_write")? != 0,
        can_execute: row.get::<_, i64>("can_execute")? != 0,
        execution_mode,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// Inserts a workspace (read-only by default) plus its permanent session grant.
pub fn insert_workspace(
    db: &Database,
    name: &str,
    root_path: &str,
) -> Result<Workspace, CoreError> {
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
        )
    })?;
    get_workspace(db, &id)
}

/// Fetches one workspace by id, or `WORKSPACE_NOT_FOUND`. Any other database
/// failure surfaces as `IO_ERROR` so a corrupt store is not misreported as
/// "not found".
pub fn get_workspace(db: &Database, id: &str) -> Result<Workspace, CoreError> {
    db.with_conn(
        |c| match c.query_row("SELECT * FROM workspaces WHERE id = ?1", [id], row_to_ws) {
            Ok(ws) => Ok(Some(ws)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        },
    )
    .and_then(|ws| {
        ws.ok_or_else(|| {
            CoreError::new(
                ErrorCode::WorkspaceNotFound,
                format!("workspace {id} not found"),
            )
        })
    })
}

/// Lists all workspaces ordered by creation time.
pub fn list_workspaces(db: &Database) -> Result<Vec<Workspace>, CoreError> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT * FROM workspaces ORDER BY created_at")?;
        let rows = stmt.query_map([], row_to_ws)?;
        rows.collect()
    })
}

/// Toggles a workspace's read/write permissions.
pub fn update_capabilities(
    db: &Database,
    id: &str,
    can_read: bool,
    can_write: bool,
) -> Result<(), CoreError> {
    let n = db.with_conn(|c| {
        c.execute(
            "UPDATE workspaces SET can_read = ?1, can_write = ?2, updated_at = ?3 WHERE id = ?4",
            rusqlite::params![can_read as i64, can_write as i64, now_iso(), id],
        )
    })?;
    if n == 0 {
        return Err(CoreError::new(
            ErrorCode::WorkspaceNotFound,
            format!("workspace {id} not found"),
        ));
    }
    Ok(())
}

/// Toggles a workspace's execute permission and execution mode, leaving
/// read/write untouched.
pub fn update_execution_access(
    db: &Database,
    id: &str,
    can_execute: bool,
    mode: ExecutionMode,
) -> Result<(), CoreError> {
    let n = db.with_conn(|c| {
        c.execute(
            "UPDATE workspaces SET can_execute = ?1, execution_mode = ?2, updated_at = ?3 WHERE id = ?4",
            rusqlite::params![can_execute as i64, mode.as_str(), now_iso(), id],
        )
    })?;
    if n == 0 {
        return Err(CoreError::new(
            ErrorCode::WorkspaceNotFound,
            format!("workspace {id} not found"),
        ));
    }
    Ok(())
}

/// Deletes a workspace. Its audit trail is untouched; the session grant
/// cascades away with it.
pub fn delete_workspace(db: &Database, id: &str) -> Result<(), CoreError> {
    let n = db.with_conn(|c| c.execute("DELETE FROM workspaces WHERE id = ?1", [id]))?;
    if n == 0 {
        return Err(CoreError::new(
            ErrorCode::WorkspaceNotFound,
            format!("workspace {id} not found"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::error::ErrorCode;

    /// Opens a fresh database in a temporary directory. The `TempDir` is
    /// returned alongside the database so it stays alive for the whole test
    /// (on Windows, dropping it while the connection is open fails).
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
        assert!(ws.can_read);
        assert!(!ws.can_write);
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
    fn get_workspace_discriminates_not_found_from_db_failure() {
        let (db, dir) = setup();
        // A malformed schema makes the SELECT fail for a non-"no rows" reason;
        // that must surface as IO_ERROR, not WORKSPACE_NOT_FOUND.
        db.with_conn(|c| c.execute("DROP TABLE workspaces", []))
            .unwrap();
        let e = get_workspace(&db, "whatever").unwrap_err();
        assert_eq!(e.code, ErrorCode::IoError);
        drop(db);
        drop(dir);
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
    fn insert_writes_permanent_session_grant() {
        let (db, _dir) = setup();
        let ws = insert_workspace(&db, "A", r"C:\a").unwrap();
        // Direct assertion on session_grants (not just the cascade side effect).
        let n = db
            .with_conn(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM session_grants WHERE workspace_id = ?1
                     AND expires_at IS NULL AND reason = 'user added via UI'",
                    [&ws.id],
                    |r| r.get::<_, i64>(0),
                )
            })
            .unwrap();
        assert_eq!(n, 1);
    }

    /// Creates a database file using the pre-execution `workspaces` schema
    /// (no `can_execute` / `execution_mode` columns) plus the matching
    /// `session_grants` table, so `Database::open` must migrate it.
    fn create_legacy_db(path: &std::path::Path, rows: &[(&str, &str)]) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE workspaces (
              id         TEXT PRIMARY KEY,
              name       TEXT NOT NULL,
              root_path  TEXT NOT NULL UNIQUE,
              can_read   INTEGER NOT NULL DEFAULT 1,
              can_write  INTEGER NOT NULL DEFAULT 0,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE session_grants (
              id           TEXT PRIMARY KEY,
              workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
              granted_at   TEXT NOT NULL,
              expires_at   TEXT,
              reason       TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
        for (id, name) in rows {
            conn.execute(
                "INSERT INTO workspaces (id, name, root_path, created_at, updated_at)
                 VALUES (?1, ?2, 'C:\\legacy', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                [id, name],
            )
            .unwrap();
        }
    }

    #[test]
    fn legacy_schema_opens_and_migrates_with_safe_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        create_legacy_db(&path, &[("w-old", "Old")]);
        let db = Database::open(&path).unwrap();
        let ws = get_workspace(&db, "w-old").unwrap();
        assert!(
            !ws.can_execute,
            "existing workspace must migrate to can_execute=false"
        );
        assert_eq!(
            ws.execution_mode,
            crate::models::ExecutionMode::ApprovedCommands,
            "existing workspace must migrate to approved_commands"
        );
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        create_legacy_db(&path, &[("w-old", "Old")]);
        let _first = Database::open(&path).unwrap();
        drop(_first);
        let second = Database::open(&path).unwrap();
        let cols: Vec<String> = second
            .with_conn(|c| {
                let mut stmt = c.prepare("PRAGMA table_info(workspaces)")?;
                let names = stmt
                    .query_map([], |r| r.get::<_, String>(1))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                Ok(names)
            })
            .unwrap();
        assert_eq!(
            cols.iter().filter(|c| *c == "can_execute").count(),
            1,
            "can_execute added exactly once"
        );
        assert_eq!(
            cols.iter().filter(|c| *c == "execution_mode").count(),
            1,
            "execution_mode added exactly once"
        );
    }

    #[test]
    fn new_workspace_defaults_read_true_write_false_execute_false() {
        let (db, _dir) = setup();
        let ws = insert_workspace(&db, "A", r"C:\a").unwrap();
        assert!(ws.can_read);
        assert!(!ws.can_write);
        assert!(!ws.can_execute);
        assert_eq!(
            ws.execution_mode,
            crate::models::ExecutionMode::ApprovedCommands
        );
    }

    #[test]
    fn update_execution_access_preserves_read_write() {
        let (db, _dir) = setup();
        let ws = insert_workspace(&db, "A", r"C:\a").unwrap();
        update_capabilities(&db, &ws.id, true, true).unwrap();
        update_execution_access(
            &db,
            &ws.id,
            true,
            crate::models::ExecutionMode::FullPowerShell,
        )
        .unwrap();
        let got = get_workspace(&db, &ws.id).unwrap();
        assert!(got.can_execute);
        assert_eq!(
            got.execution_mode,
            crate::models::ExecutionMode::FullPowerShell
        );
        assert!(got.can_read, "changing execute must not touch read");
        assert!(got.can_write, "changing execute must not touch write");
    }

    #[test]
    fn unknown_stored_execution_mode_is_corruption_error() {
        let (db, _dir) = setup();
        let ws = insert_workspace(&db, "A", r"C:\a").unwrap();
        db.with_conn(|c| {
            c.execute(
                "UPDATE workspaces SET execution_mode = 'bogus' WHERE id = ?1",
                [&ws.id],
            )
        })
        .unwrap();
        let e = get_workspace(&db, &ws.id).unwrap_err();
        assert_eq!(e.code, ErrorCode::IoError);
        assert!(
            e.message.contains("execution_mode"),
            "corruption error must name the bad column, got: {}",
            e.message
        );
    }

    #[test]
    fn audit_events_is_append_only() {
        let (db, _dir) = setup();
        db.with_conn(|c| {
            c.execute("INSERT INTO audit_events (ts, source, op, decision) VALUES ('t','ui','op','allow')", [])
        }).unwrap();
        let upd = db.with_conn(|c| c.execute("UPDATE audit_events SET decision='deny'", []));
        assert!(upd.is_err(), "UPDATE on audit_events must be rejected");
        let del = db.with_conn(|c| c.execute("DELETE FROM audit_events", []));
        assert!(del.is_err(), "DELETE on audit_events must be rejected");
    }
}
