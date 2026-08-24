//! Fail-closed policy pipeline: the single auditor through which every MCP
//! request flows. Exactly one audit row is written per invocation.

use crate::audit;
use crate::error::{CoreError, ErrorCode};
use crate::models::Workspace;
use crate::workspace_repo::get_workspace;
use crate::CoreServices;
use std::path::{Path, PathBuf};

/// Safe per-operation audit metadata carried alongside an op result.
/// `path` is a workspace-relative request path and `detail` holds bounded
/// metrics — never file contents, matched search lines, or secrets.
pub struct Audited<T> {
    pub value: T,
    pub path: Option<String>,
    pub detail: Option<serde_json::Value>,
}

impl<T> Audited<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            path: None,
            detail: None,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }
}

/// The capability an operation requires from its workspace.
/// `Read`/`Write` gate filesystem tools; `Execute` gates command execution
/// and requires only `can_execute` (read/write are not implied).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredCapability {
    Read,
    Write,
    Execute,
}

impl RequiredCapability {
    /// Legacy boolean mapping kept for the existing MCP call sites:
    /// `false` -> Read-only tool, `true` -> writing tool.
    fn from_needs_write(needs_write: bool) -> Self {
        if needs_write {
            Self::Write
        } else {
            Self::Read
        }
    }

    fn check(self, ws: &Workspace) -> Result<(), CoreError> {
        match self {
            Self::Read => {
                if !ws.can_read {
                    return Err(CoreError::new(
                        ErrorCode::ReadDisabled,
                        format!("workspace '{}' has read access disabled", ws.name),
                    ));
                }
                Ok(())
            }
            Self::Write => {
                if !ws.can_write {
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
                Ok(())
            }
            // Execute stands alone: it must not silently inherit read/write.
            Self::Execute => {
                if !ws.can_execute {
                    return Err(CoreError::new(
                        ErrorCode::ExecuteDisabled,
                        format!("workspace '{}' has command execution disabled", ws.name),
                    ));
                }
                Ok(())
            }
        }
    }
}

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
    f: impl FnOnce(&Workspace, &Path) -> Result<Audited<T>, CoreError>,
) -> Result<T, CoreError> {
    run_tool_audited_with_context(core, tool, workspace_id, needs_write, None, None, f)
}

/// Capability-explicit entry point for future execution tools: same
/// fail-closed pipeline as [`run_tool_audited_with_context`], but the caller
/// names the required [`RequiredCapability`] directly instead of using the
/// legacy read/write boolean mapping.
pub fn run_tool_with_capability<T>(
    core: &CoreServices,
    tool: &str,
    workspace_id: &str,
    capability: RequiredCapability,
    path: Option<&str>,
    detail: Option<serde_json::Value>,
    f: impl FnOnce(&Workspace, &Path) -> Result<Audited<T>, CoreError>,
) -> Result<T, CoreError> {
    let pre = (|| -> Result<(Workspace, PathBuf), CoreError> {
        if core.pause.is_paused() {
            return Err(CoreError::new(
                ErrorCode::AccessPaused,
                "access is paused by the user",
            ));
        }
        let ws = get_workspace(&core.db, workspace_id)?;
        capability.check(&ws)?;
        let root = ws.root_path.clone();
        Ok((ws, root))
    })();

    let (ws, root) = match pre {
        Ok(ok) => ok,
        Err(e) => {
            let known_ws = matches!(
                e.code,
                ErrorCode::ReadDisabled | ErrorCode::WriteDisabled | ErrorCode::ExecuteDisabled
            );
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
                path,
                detail.clone(),
            )?;
            return Err(e);
        }
    };

    let audited = f(&ws, &root);
    // Op-supplied metadata wins; caller-provided context is the fallback
    // (and the only source on error rows, where the op never ran).
    let (op_path, op_detail) = match &audited {
        Ok(a) => (
            a.path.as_deref().or(path),
            a.detail.clone().or_else(|| detail.clone()),
        ),
        Err(_) => (path, detail),
    };
    audit::record_simple(
        &core.db,
        "mcp",
        Some(tool),
        Some(workspace_id),
        tool,
        if audited.is_ok() { "allow" } else { "error" },
        audited.as_ref().err().map(|e| e.code.as_str()),
        op_path,
        op_detail,
    )?;
    audited.map(|a| a.value)
}

/// Same fail-closed pipeline with bounded, caller-provided fallback audit
/// context used for the prelude deny/error rows (before the op has run and can
/// supply its own [`Audited`] metadata). `path` is a workspace-relative
/// request path/prefix, never file content.
///
/// The boolean maps to capabilities (`false` -> Read, `true` -> Write); use
/// [`run_tool_with_capability`] when a different capability is required.
pub fn run_tool_audited_with_context<T>(
    core: &CoreServices,
    tool: &str,
    workspace_id: &str,
    needs_write: bool,
    path: Option<&str>,
    detail: Option<serde_json::Value>,
    f: impl FnOnce(&Workspace, &Path) -> Result<Audited<T>, CoreError>,
) -> Result<T, CoreError> {
    run_tool_with_capability(
        core,
        tool,
        workspace_id,
        RequiredCapability::from_needs_write(needs_write),
        path,
        detail,
        f,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::error::ErrorCode;
    use crate::models::ExecutionMode;
    use crate::pause::PauseFlag;
    use crate::sensitive::SensitiveMatcher;
    use crate::workspace_repo::{insert_workspace, update_capabilities, update_execution_access};
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
            |_ws, _root| -> Result<Audited<()>, CoreError> {
                panic!("closure must not run while paused")
            },
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
            |_ws, _r| -> Result<Audited<()>, CoreError> {
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
            |_ws, _r| -> Result<Audited<()>, CoreError> {
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
            |_ws, _r| -> Result<Audited<()>, CoreError> {
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
            Ok(Audited::new(42))
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
    fn contextual_audit_records_path_and_bounded_detail() {
        let (core, _d, ws_id) = setup();
        run_tool_audited_with_context(
            &core,
            "read_file",
            &ws_id,
            false,
            Some("src/main.rs"),
            Some(serde_json::json!({"window": "partial"})),
            |_ws, _root| Ok(Audited::new(())),
        )
        .unwrap();
        let events = crate::audit::list_recent(&core.db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].path.as_deref(), Some("src/main.rs"));
        assert_eq!(events[0].detail.as_ref().unwrap()["window"], "partial");
    }

    #[test]
    fn closure_error_audited_as_error() {
        let (core, _d, ws_id) = setup();
        let e = run_tool_audited(
            &core,
            "read_file",
            &ws_id,
            false,
            |_ws, _r| -> Result<Audited<i32>, CoreError> {
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
    fn execute_requires_can_execute_only() {
        let (core, _d, ws_id) = setup(); // can_execute defaults false
        let e = run_tool_with_capability(
            &core,
            "run_command",
            &ws_id,
            RequiredCapability::Execute,
            None,
            None,
            |_ws, _r| -> Result<Audited<()>, CoreError> {
                panic!("closure must not run without execute capability")
            },
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::ExecuteDisabled);
        let events = crate::audit::list_recent(&core.db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decision, "deny");
        assert_eq!(events[0].error_code.as_deref(), Some("EXECUTE_DISABLED"));
        assert_eq!(events[0].workspace_id.as_deref(), Some(ws_id.as_str()));
    }

    #[test]
    fn execute_does_not_require_read_or_write() {
        let (core, _d, ws_id) = setup();
        // No read, no write — but execute granted. Execute stands alone.
        update_capabilities(&core.db, &ws_id, false, false).unwrap();
        update_execution_access(&core.db, &ws_id, true, ExecutionMode::FullPowerShell).unwrap();
        let v = run_tool_with_capability(
            &core,
            "run_command",
            &ws_id,
            RequiredCapability::Execute,
            None,
            None,
            |_ws, _r| Ok(Audited::new("ran")),
        )
        .unwrap();
        assert_eq!(v, "ran");
    }

    #[test]
    fn paused_blocks_execute_too() {
        let (core, _d, ws_id) = setup();
        update_execution_access(&core.db, &ws_id, true, ExecutionMode::ApprovedCommands).unwrap();
        core.pause.set(true);
        let e = run_tool_with_capability(
            &core,
            "run_command",
            &ws_id,
            RequiredCapability::Execute,
            None,
            None,
            |_ws, _r| -> Result<Audited<()>, CoreError> { panic!("paused must win") },
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::AccessPaused);
    }

    #[test]
    fn exactly_one_row_per_invocation() {
        let (core, _d, ws_id) = setup();
        run_tool_audited(&core, "read_file", &ws_id, false, |_ws, _r| {
            Ok(Audited::new(()))
        })
        .unwrap();
        let _ = run_tool_audited(&core, "read_file", "ghost", false, |_ws, _r| {
            Ok(Audited::new(()))
        });
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
            Ok(Audited::new("closure ran"))
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
            |_ws, _r| -> Result<Audited<()>, CoreError> { panic!("must not run") },
        )
        .unwrap_err();
        // Fail-closed: the deny is not swallowed — writing its audit row also
        // failed, so the pipeline surfaces IO_ERROR instead of the pause deny.
        assert_eq!(e.code, ErrorCode::IoError);
    }
}
