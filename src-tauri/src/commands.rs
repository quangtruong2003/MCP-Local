//! Tauri IPC commands backing the manager UI.
//!
//! Every mutating command writes an audit row with `source: "ui"` and
//! returns `Result<T, String>` where the string is serialized CoreError
//! JSON (stable `code` + `message`).

use serde::Serialize;
use tauri::{Manager, State};

use crate::{
    connection::{
        manager::{ConnectionError, ConnectionManager as TailscaleManager},
        ConnectionStatus,
    },
    settings, AppState,
};
use lc_core::{audit, error::CoreError, models::AuditEvent, workspace_repo};

fn err_string(e: CoreError) -> String {
    serde_json::json!({
        "code": e.code.as_str(),
        "message": e.message,
        "detail": e.detail,
    })
    .to_string()
}

/// Serializes a [`ConnectionError`] into the same stable `{code,message}`
/// JSON shape the UI already parses. Never carries the capability token.
fn conn_err_string(e: ConnectionError) -> String {
    serde_json::json!({
        "code": e.code,
        "message": e.message,
    })
    .to_string()
}

fn settings_err(code: &str, message: impl std::fmt::Display) -> String {
    serde_json::json!({ "code": code, "message": message.to_string() }).to_string()
}

/// Validates a Tailscale Funnel hostname label: 1-63 characters, lowercase
/// ASCII letters / digits / hyphens only, starting and ending with a letter
/// or digit. On failure the error string uses the stable JSON shape with
/// code `HOSTNAME_INVALID`.
pub(crate) fn validate_hostname(hostname: &str) -> Result<(), String> {
    let valid = !hostname.is_empty()
        && hostname.len() <= 63
        && hostname
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && hostname
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && hostname
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(settings_err(
            "HOSTNAME_INVALID",
            format!("invalid hostname '{hostname}': use 1-63 lowercase letters, digits, or hyphens, starting and ending with a letter or digit"),
        ))
    }
}

#[derive(Serialize)]
pub struct Status {
    pub port: u16,
    pub paused: bool,
}

/// Records a UI-source audit row; a failed audit write is logged loudly instead of
/// silently swallowed (the mutation itself has already succeeded and is returned).
fn audit_ui(
    db: &lc_core::db::Database,
    workspace_id: Option<&str>,
    op: &str,
    error_code: Option<&str>,
    path: Option<&str>,
    detail: Option<serde_json::Value>,
) {
    if let Err(e) = audit::record_simple(
        db,
        "ui",
        None,
        workspace_id,
        op,
        "allow",
        error_code,
        path,
        detail,
    ) {
        eprintln!("audit write failed for ui op {op}: {e}");
    }
}

#[tauri::command]
pub fn add_workspace(
    state: State<'_, AppState>,
    root_path: String,
    name: Option<String>,
) -> Result<lc_core::models::Workspace, String> {
    let canonical = dunce::canonicalize(&root_path).map_err(|e| {
        err_string(CoreError::new(
            lc_core::error::ErrorCode::InvalidArgument,
            format!("invalid path {root_path}: {e}"),
        ))
    })?;
    if !canonical.is_dir() {
        return Err(err_string(CoreError::new(
            lc_core::error::ErrorCode::InvalidArgument,
            format!("{root_path} is not a directory"),
        )));
    }
    let ws = workspace_repo::insert_workspace(
        &state.core.db,
        &name.unwrap_or_else(|| {
            canonical.file_name().map_or_else(
                || canonical.display().to_string(),
                |f| f.to_string_lossy().into_owned(),
            )
        }),
        &canonical.display().to_string(),
    )
    .map_err(err_string)?;
    audit_ui(
        &state.core.db,
        Some(&ws.id),
        "add_workspace",
        None,
        Some(&canonical.display().to_string()),
        None,
    );
    Ok(ws)
}

#[tauri::command]
pub fn list_workspaces(
    state: State<'_, AppState>,
) -> Result<Vec<lc_core::models::Workspace>, String> {
    workspace_repo::list_workspaces(&state.core.db).map_err(err_string)
}

#[tauri::command]
pub fn set_capabilities(
    state: State<'_, AppState>,
    id: String,
    can_read: bool,
    can_write: bool,
) -> Result<(), String> {
    workspace_repo::update_capabilities(&state.core.db, &id, can_read, can_write)
        .map_err(err_string)?;
    audit_ui(
        &state.core.db,
        Some(&id),
        "set_capabilities",
        None,
        None,
        None,
    );
    Ok(())
}

/// Toggles a workspace's execute permission and execution mode. Read/write
/// capabilities are untouched.
#[tauri::command]
pub fn set_execution_access(
    state: State<'_, AppState>,
    id: String,
    can_execute: bool,
    execution_mode: lc_core::models::ExecutionMode,
) -> Result<(), String> {
    lc_core::workspace_repo::update_execution_access(
        &state.core.db,
        &id,
        can_execute,
        execution_mode,
    )
    .map_err(err_string)?;
    audit_ui(
        &state.core.db,
        Some(&id),
        "set_execution_access",
        None,
        None,
        None,
    );
    Ok(())
}

/// Removes the workspace registration; the files on disk are untouched.
#[tauri::command]
pub fn remove_workspace(state: State<'_, AppState>, id: String) -> Result<(), String> {
    workspace_repo::delete_workspace(&state.core.db, &id).map_err(err_string)?;
    audit_ui(
        &state.core.db,
        Some(&id),
        "remove_workspace",
        None,
        None,
        None,
    );
    Ok(())
}

#[tauri::command]
pub fn set_paused(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    paused: bool,
) -> Result<(), String> {
    state.core.pause.set(paused);
    audit_ui(
        &state.core.db,
        None,
        if paused { "pause" } else { "resume" },
        None,
        None,
        None,
    );
    use tauri::Emitter;
    app.emit("access-paused", paused).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn recent_audit(state: State<'_, AppState>) -> Result<Vec<AuditEvent>, String> {
    audit::list_recent(&state.core.db, 100).map_err(err_string)
}

#[tauri::command]
pub fn server_status(state: State<'_, AppState>) -> Status {
    Status {
        port: state.port,
        paused: state.core.pause.is_paused(),
    }
}

/// Thin wrapper over the Tailscale [`TailscaleManager`]: current status.
/// Async commands taking `State<'_, _>` must return `Result`, hence the
/// infallible `Ok`.
#[tauri::command]
pub async fn connection_status(
    manager: State<'_, TailscaleManager>,
) -> Result<ConnectionStatus, String> {
    Ok(manager.status().await)
}

/// Starts the funnel connection via the new manager.
#[tauri::command]
pub async fn start_connection(
    manager: State<'_, TailscaleManager>,
) -> Result<ConnectionStatus, String> {
    manager.start().await.map_err(conn_err_string)
}

/// Stops the funnel connection via the new manager.
#[tauri::command]
pub async fn stop_connection(
    manager: State<'_, TailscaleManager>,
) -> Result<ConnectionStatus, String> {
    manager.stop().await.map_err(conn_err_string)
}

/// Stop then start: retries the funnel connection.
#[tauri::command]
pub async fn retry_connection(
    manager: State<'_, TailscaleManager>,
) -> Result<ConnectionStatus, String> {
    manager.retry().await.map_err(conn_err_string)
}

/// Rotates the capability secret (restarts when running). Never returns the token.
#[tauri::command]
pub async fn rotate_connection_url(
    manager: State<'_, TailscaleManager>,
) -> Result<ConnectionStatus, String> {
    manager.rotate_capability().await.map_err(conn_err_string)
}

/// Resets Tailscale identity: stops, optionally removes the Tailscale state
/// directory, rotates the capability secret, and restarts only if asked.
#[tauri::command]
pub async fn reset_tailscale_identity(
    remove_state_dir: bool,
    restart: bool,
    manager: State<'_, TailscaleManager>,
) -> Result<ConnectionStatus, String> {
    manager
        .reset_identity(remove_state_dir, restart)
        .await
        .map_err(conn_err_string)
}

#[tauri::command]
pub fn get_app_settings(app: tauri::AppHandle) -> Result<settings::AppSettings, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    settings::load_app_settings(&dir).map_err(|e| settings_err("SETTINGS_INVALID", e))
}

/// Validates then persists application settings; rejects an invalid
/// `tailscale_hostname` before touching disk.
#[tauri::command]
pub fn update_app_settings(
    app: tauri::AppHandle,
    settings: settings::AppSettings,
) -> Result<settings::AppSettings, String> {
    validate_hostname(&settings.tailscale_hostname)?;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    settings::save_app_settings(&dir, &settings)
        .map_err(|e| settings_err("SETTINGS_SAVE_FAILED", e))?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_hostname_accepts_valid_labels() {
        for ok in [
            "local-workspace-mcp",
            "a",
            "0",
            "abc123",
            "a-b-c",
            "1machine2",
            &"x".repeat(63),
        ] {
            assert!(validate_hostname(ok).is_ok(), "'{ok}' should be valid");
        }
    }

    #[test]
    fn validate_hostname_rejects_empty() {
        let err = validate_hostname("").unwrap_err();
        assert!(err.contains("HOSTNAME_INVALID"));
        let parsed: serde_json::Value = serde_json::from_str(&err).unwrap();
        assert_eq!(parsed["code"], "HOSTNAME_INVALID");
    }

    #[test]
    fn validate_hostname_rejects_over_63_chars() {
        assert!(validate_hostname(&"x".repeat(64)).is_err());
    }

    #[test]
    fn validate_hostname_rejects_uppercase() {
        assert!(validate_hostname("Local-Workspace").is_err());
    }

    #[test]
    fn validate_hostname_rejects_leading_and_trailing_hyphen() {
        assert!(validate_hostname("-lead").is_err());
        assert!(validate_hostname("trail-").is_err());
    }

    #[test]
    fn validate_hostname_rejects_underscore() {
        assert!(validate_hostname("under_score").is_err());
    }

    #[test]
    fn validate_hostname_rejects_spaces() {
        assert!(validate_hostname("has space").is_err());
    }

    #[test]
    fn conn_error_serializes_to_stable_code_message_shape() {
        let e = ConnectionError {
            code: "SIDECAR_NOT_FOUND".into(),
            message: "sidecar binary missing".into(),
        };
        let parsed: serde_json::Value = serde_json::from_str(&conn_err_string(e)).unwrap();
        let obj = parsed.as_object().unwrap();
        assert_eq!(obj.len(), 2, "shape must be exactly {{code,message}}");
        assert_eq!(parsed["code"], "SIDECAR_NOT_FOUND");
        assert_eq!(parsed["message"], "sidecar binary missing");
    }
}
