//! Tauri IPC commands backing the manager UI.
//!
//! Every mutating command writes an audit row with `source: "ui"` and
//! returns `Result<T, String>` where the string is serialized CoreError
//! JSON (stable `code` + `message`).

use serde::Serialize;
use tauri::{Manager, State};

use crate::{settings, tunnel, AppState};
use lc_core::{audit, error::CoreError, models::AuditEvent, workspace_repo};

fn err_string(e: CoreError) -> String {
    serde_json::json!({
        "code": e.code.as_str(),
        "message": e.message,
        "detail": e.detail,
    })
    .to_string()
}

#[derive(Serialize)]
pub struct Status {
    pub port: u16,
    pub paused: bool,
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
        &name.unwrap_or_else(|| canonical.display().to_string()),
        &canonical.display().to_string(),
    )
    .map_err(err_string)?;
    let _ = audit::record_simple(
        &state.core.db,
        "ui",
        None,
        Some(&ws.id),
        "add_workspace",
        "allow",
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
    let _ = audit::record_simple(
        &state.core.db,
        "ui",
        None,
        Some(&id),
        "set_capabilities",
        "allow",
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
    let _ = audit::record_simple(
        &state.core.db,
        "ui",
        None,
        Some(&id),
        "remove_workspace",
        "allow",
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
    let _ = audit::record_simple(
        &state.core.db,
        "ui",
        None,
        None,
        if paused { "pause" } else { "resume" },
        "allow",
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

/// Persists tunnel credentials and (re)starts the sidecar.
#[tauri::command]
pub fn configure_tunnel(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    tunnel_id: String,
    api_key: String,
) -> Result<(), String> {
    // Always stop any running sidecar first — even when the new config is
    // empty (disabling the tunnel must not leave a stale process bridged).
    tunnel::stop(&app);
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let saved = settings::Settings {
        tunnel_id,
        tunnel_api_key: api_key,
    };
    settings::save(&dir, &saved)?;
    if saved.tunnel_id.is_empty() {
        return Ok(());
    }
    tunnel::spawn(&app, &saved.tunnel_id, &saved.tunnel_api_key, state.port)
}

/// Stops the sidecar; persisted settings are left untouched so the user can
/// re-enable later without re-entering credentials.
#[tauri::command]
pub fn stop_tunnel(app: tauri::AppHandle) {
    tunnel::stop(&app);
}

/// Current sidecar status as serialized `TunnelStatus` JSON.
#[tauri::command]
pub fn tunnel_status(app: tauri::AppHandle) -> String {
    let handle = app.state::<crate::tunnel::TunnelHandle>();
    let status = handle.status.lock().unwrap().clone();
    serde_json::to_string(&status).unwrap_or_else(|_| "\"stopped\"".into())
}
