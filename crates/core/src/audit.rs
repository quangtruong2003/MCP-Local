//! Append-only audit log: recording events and listing recent entries.

use crate::db::Database;
use crate::error::CoreError;
use crate::models::AuditEvent;

/// Inserts an audit event. `detail` is stored as a JSON string; `id` is
/// assigned by the database and ignored on input.
pub fn record(db: &Database, ev: &AuditEvent) -> Result<(), CoreError> {
    let id = db.with_conn(|c| {
        c.execute(
            "INSERT INTO audit_events (ts, source, tool, workspace_id, op, decision, error_code, path, detail)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![
                ev.ts,
                ev.source,
                ev.tool,
                ev.workspace_id,
                ev.op,
                ev.decision,
                ev.error_code,
                ev.path,
                ev.detail.as_ref().map(|d| d.to_string()),
            ],
        )?;
        Ok(c.last_insert_rowid())
    })?;
    let mut persisted = ev.clone();
    persisted.id = id;
    db.notify_audit_listener(&persisted);
    Ok(())
}

/// Convenience wrapper that fills `ts` with the current UTC RFC 3339 time.
#[allow(clippy::too_many_arguments)]
pub fn record_simple(
    db: &Database,
    source: &str,
    tool: Option<&str>,
    workspace_id: Option<&str>,
    op: &str,
    decision: &str,
    error_code: Option<&str>,
    path: Option<&str>,
    detail: Option<serde_json::Value>,
) -> Result<(), CoreError> {
    record(
        db,
        &AuditEvent {
            id: 0,
            ts: now_iso(),
            source: source.into(),
            tool: tool.map(Into::into),
            workspace_id: workspace_id.map(Into::into),
            op: op.into(),
            decision: decision.into(),
            error_code: error_code.map(Into::into),
            path: path.map(Into::into),
            detail,
        },
    )
}

/// Returns the most recent audit events, newest first.
pub fn list_recent(db: &Database, limit: u32) -> Result<Vec<AuditEvent>, CoreError> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT * FROM audit_events ORDER BY id DESC LIMIT ?1")?;
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
                detail: row
                    .get::<_, Option<String>>("detail")?
                    .and_then(|s| serde_json::from_str(&s).ok()),
            })
        })?;
        rows.collect()
    })
}

fn now_iso() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

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
        record_simple(
            &db,
            "mcp",
            Some("read_file"),
            Some("w1"),
            "read_file",
            "allow",
            None,
            Some(r"C:\w\f.txt"),
            Some(serde_json::json!({"bytes": 10})),
        )
        .unwrap();
        let events = list_recent(&db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decision, "allow");
        assert_eq!(events[0].detail.as_ref().unwrap()["bytes"], 10);
        assert!(!events[0].ts.is_empty());
    }

    #[test]
    fn newest_first_and_limit() {
        let (db, _d) = setup();
        for i in 0..5 {
            record_simple(
                &db,
                "mcp",
                None,
                None,
                &format!("op{i}"),
                "allow",
                None,
                None,
                None,
            )
            .unwrap();
        }
        let events = list_recent(&db, 3).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].op, "op4");
    }

    #[test]
    fn successful_record_notifies_listener_with_persisted_event() {
        let (db, _d) = setup();
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let listener_received = received.clone();
        db.set_audit_listener(std::sync::Arc::new(move |event: &AuditEvent| {
            listener_received.lock().unwrap().push(event.clone());
        }));

        record_simple(
            &db,
            "mcp",
            Some("read_file"),
            Some("w1"),
            "read_file",
            "allow",
            None,
            Some("notes.txt"),
            None,
        )
        .unwrap();

        let received = received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert!(received[0].id > 0);
        assert_eq!(received[0].op, "read_file");
        assert_eq!(list_recent(&db, 1).unwrap()[0].id, received[0].id);
    }

    #[test]
    fn failed_record_does_not_notify_listener() {
        let (db, _d) = setup();
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let listener_received = received.clone();
        db.set_audit_listener(std::sync::Arc::new(move |event: &AuditEvent| {
            listener_received.lock().unwrap().push(event.clone());
        }));

        // Break the insert path so persistence fails (fail-closed).
        db.with_conn(|c| c.execute_batch("DROP TABLE audit_events"))
            .unwrap();

        let result = record_simple(&db, "mcp", None, None, "op", "allow", None, None, None);
        assert!(result.is_err());
        assert!(received.lock().unwrap().is_empty());
    }
}
