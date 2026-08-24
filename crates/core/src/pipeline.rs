//! Fail-closed policy pipeline: the single auditor through which every MCP
//! request flows. Exactly one audit row is written per invocation.

use crate::audit;
use crate::error::{CoreError, ErrorCode};
use crate::models::Workspace;
use crate::workspace_repo::get_workspace;
use crate::CoreServices;
use std::path::{Path, PathBuf};

/// Runs one tool call through the fail-closed pipeline and writes exactly one
/// audit row:
///
/// 1. Pause check (kill switch consulted first) -> `ACCESS_PAUSED` deny row,
///    workspace_id NULL.
/// 2. Workspace lookup -> `WORKSPACE_NOT_FOUND` error row, workspace_id NULL.
/// 3. Capability check -> `WRITE_DISABLED` / `READ_DISABLED` deny rows with
///    the workspace_id once known.
/// 4. Closure `f` (confinement + op validation + execution):
///    Ok -> allow row; Err -> error row. All rows carry workspace_id.
///
/// This is the only place audit rows are written for tool calls.
pub fn run_tool_audited<T>(
    core: &CoreServices,
    tool: &str,
    workspace_id: &str,
    needs_write: bool,
    f: impl FnOnce(&Workspace, &Path) -> Result<T, CoreError>,
) -> Result<T, CoreError> {
    let pre = (|| -> Result<(Workspace, PathBuf), CoreError> {
        if core.pause.is_paused() {
            return Err(CoreError::new(
                ErrorCode::AccessPaused,
                "access is paused by the user",
            ));
        }
        let ws = get_workspace(&core.db, workspace_id)?;
        if needs_write && !ws.can_write {
            return Err(CoreError::new(
                ErrorCode::WriteDisabled,
                format!("workspace '{}' has write access disabled", ws.name),
            ));
        }
        if !ws.can_read {
            return Err(CoreError::new(
                ErrorCode::ReadDisabled,
                format!("workspace '{}' has read access disabled", ws.name),
            ));
        }
        let root = ws.root_path.clone();
        Ok((ws, root))
    })();

    let (ws, root) = match pre {
        Ok(ok) => ok,
        Err(e) => {
            // Deny codes happen before execution; anything else from the
            // prelude (e.g. a db failure surfacing as WORKSPACE_NOT_FOUND or
            // IO_ERROR) is an error. Capability denies know the workspace;
            // pause and not-found do not.
            let known_ws = matches!(e.code, ErrorCode::ReadDisabled | ErrorCode::WriteDisabled);
            let decision = if known_ws || e.code == ErrorCode::AccessPaused {
                "deny"
            } else {
                "error"
            };
            audit::record_simple(
                &core.db,
                "mcp",
                Some(tool),
                if known_ws { Some(workspace_id) } else { None },
                tool,
                decision,
                Some(e.code.as_str()),
                None,
                None,
            )?;
            return Err(e);
        }
    };

    let result = f(&ws, &root);
    audit::record_simple(
        &core.db,
        "mcp",
        Some(tool),
        Some(workspace_id),
        tool,
        if result.is_ok() { "allow" } else { "error" },
        result.as_ref().err().map(|e| e.code.as_str()),
        None,
        None,
    )?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::error::ErrorCode;
    use crate::pause::PauseFlag;
    use crate::sensitive::SensitiveMatcher;
    use crate::workspace_repo::{insert_workspace, update_capabilities};
    use std::fs;

    fn setup() -> (CoreServices, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("t.db")).unwrap();
        fs::create_dir_all(dir.path().join("ws")).unwrap();
        let ws = insert_workspace(&db, "W", &dir.path().join("ws").to_string_lossy()).unwrap();
        let core = CoreServices {
            db,
            pause: PauseFlag::default(),
            sensitive: SensitiveMatcher::new(),
        };
        (core, dir, ws.id)
    }

    #[test]
    fn paused_blocks_everything() {
        let (core, _d, ws_id) = setup();
        core.pause.set(true);
        let e = run_tool_audited(
            &core,
            "list_directory",
            &ws_id,
            false,
            |_ws, _root| -> Result<(), CoreError> { panic!("closure must not run while paused") },
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::AccessPaused);
        let events = crate::audit::list_recent(&core.db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].error_code.as_deref(), Some("ACCESS_PAUSED"));
        assert_eq!(events[0].decision, "deny");
        assert_eq!(events[0].workspace_id, None);
    }

    #[test]
    fn unknown_workspace_denied_and_audited() {
        let (core, _d, _id) = setup();
        let e = run_tool_audited(
            &core,
            "read_file",
            "ghost",
            false,
            |_ws, _r| -> Result<(), CoreError> {
                panic!("closure must not run for unknown workspace")
            },
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::WorkspaceNotFound);
        let events = crate::audit::list_recent(&core.db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decision, "error");
        assert_eq!(events[0].error_code.as_deref(), Some("WORKSPACE_NOT_FOUND"));
        assert_eq!(events[0].workspace_id, None);
    }

    #[test]
    fn write_needs_write_capability() {
        let (core, _d, ws_id) = setup(); // can_write defaults false
        let e = run_tool_audited(
            &core,
            "apply_patch",
            &ws_id,
            true,
            |_ws, _r| -> Result<(), CoreError> {
                panic!("closure must not run without write capability")
            },
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::WriteDisabled);
        let events = crate::audit::list_recent(&core.db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decision, "deny");
        assert_eq!(events[0].workspace_id.as_deref(), Some(ws_id.as_str()));
    }

    #[test]
    fn read_needs_read_capability() {
        let (core, _d, ws_id) = setup();
        update_capabilities(&core.db, &ws_id, false, false).unwrap();
        let e = run_tool_audited(
            &core,
            "read_file",
            &ws_id,
            false,
            |_ws, _r| -> Result<(), CoreError> {
                panic!("closure must not run without read capability")
            },
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::ReadDisabled);
        let events = crate::audit::list_recent(&core.db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decision, "deny");
        assert_eq!(events[0].workspace_id.as_deref(), Some(ws_id.as_str()));
    }

    #[test]
    fn success_audits_allow_with_workspace() {
        let (core, dir, ws_id) = setup();
        let v = run_tool_audited(&core, "read_file", &ws_id, false, |ws, root| {
            assert!(ws.can_read);
            assert_eq!(root, dir.path().join("ws"));
            Ok(42)
        })
        .unwrap();
        assert_eq!(v, 42);
        let events = crate::audit::list_recent(&core.db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decision, "allow");
        assert_eq!(events[0].workspace_id.as_deref(), Some(ws_id.as_str()));
        assert_eq!(events[0].error_code, None);
    }

    #[test]
    fn closure_error_audited_as_error() {
        let (core, _d, ws_id) = setup();
        let e = run_tool_audited(
            &core,
            "read_file",
            &ws_id,
            false,
            |_ws, _r| -> Result<i32, CoreError> {
                Err(CoreError::new(ErrorCode::FileNotFound, "x"))
            },
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::FileNotFound);
        let events = crate::audit::list_recent(&core.db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decision, "error");
        assert_eq!(events[0].error_code.as_deref(), Some("FILE_NOT_FOUND"));
        assert_eq!(events[0].workspace_id.as_deref(), Some(ws_id.as_str()));
    }

    #[test]
    fn exactly_one_row_per_invocation() {
        let (core, _d, ws_id) = setup();
        run_tool_audited(&core, "read_file", &ws_id, false, |_ws, _r| Ok(())).unwrap();
        let _ = run_tool_audited(&core, "read_file", "ghost", false, |_ws, _r| Ok(()));
        let events = crate::audit::list_recent(&core.db, 10).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn allow_is_fail_closed_when_audit_write_fails() {
        let (core, _d, ws_id) = setup();
        // Break the audit sink: the allow row cannot be written.
        core.db
            .with_conn(|c| c.execute("DROP TABLE audit_events", []))
            .unwrap();
        let e = run_tool_audited(&core, "read_file", &ws_id, false, |_ws, _r| {
            Ok("closure ran")
        })
        .unwrap_err();
        // Fail-closed: even though the closure succeeded, the caller gets an error.
        assert_eq!(e.code, ErrorCode::IoError);
    }

    #[test]
    fn deny_is_fail_closed_when_audit_write_fails() {
        let (core, _d, ws_id) = setup();
        core.pause.set(true);
        core.db
            .with_conn(|c| c.execute("DROP TABLE audit_events", []))
            .unwrap();
        let e = run_tool_audited(
            &core,
            "read_file",
            &ws_id,
            false,
            |_ws, _r| -> Result<(), CoreError> { panic!("must not run") },
        )
        .unwrap_err();
        // Fail-closed: the deny is not swallowed — writing its audit row also
        // failed, so the pipeline surfaces IO_ERROR instead of the pause deny.
        assert_eq!(e.code, ErrorCode::IoError);
    }
}
