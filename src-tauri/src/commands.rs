//! Tauri IPC commands backing the manager UI.
//!
//! Every mutating command writes an audit row with `source: "ui"` and
//! returns `Result<T, String>` where the string is serialized CoreError
//! JSON (stable `code` + `message`).

use serde::Serialize;
use tauri::{Manager, State};

use crate::{
    connection::{ConnectionManager, ConnectionSnapshot},
    secret_store::{SecretStore, SecretSummary},
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
    if let Err(e) = audit::record_simple(db, "ui", None, workspace_id, op, "allow", error_code, path, detail) {
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
        &name.unwrap_or_else(|| canonical.file_name().map_or_else(
            || canonical.display().to_string(),
            |f| f.to_string_lossy().into_owned(),
        )),
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
    audit_ui(&state.core.db, Some(&id), "set_capabilities", None, None, None);
    Ok(())
}

/// Removes the workspace registration; the files on disk are untouched.
#[tauri::command]
pub fn remove_workspace(state: State<'_, AppState>, id: String) -> Result<(), String> {
    workspace_repo::delete_workspace(&state.core.db, &id).map_err(err_string)?;
    audit_ui(&state.core.db, Some(&id), "remove_workspace", None, None, None);
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

#[derive(Debug, Serialize)]
pub struct ConnectionSettingsSummary {
    pub tunnel_id: String,
    pub organization_id: Option<String>,
    pub workspace_id: Option<String>,
    pub tunnel_name: String,
    pub auto_start: bool,
    pub has_runtime_key: bool,
    pub has_admin_key: bool,
}

fn settings_summary(
    settings: settings::ConnectionSettings,
    secret_summary: SecretSummary,
) -> ConnectionSettingsSummary {
    ConnectionSettingsSummary {
        tunnel_id: settings.tunnel_id,
        organization_id: settings.organization_id,
        workspace_id: settings.workspace_id,
        tunnel_name: settings.tunnel_name,
        auto_start: settings.auto_start,
        has_runtime_key: secret_summary.has_runtime_key,
        has_admin_key: secret_summary.has_admin_key,
    }
}

#[tauri::command]
pub fn connection_status(manager: State<'_, ConnectionManager>) -> ConnectionSnapshot {
    manager.snapshot()
}

#[tauri::command]
pub async fn start_connection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    manager: State<'_, ConnectionManager>,
) -> Result<ConnectionSnapshot, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    manager.start(&app, &dir, state.port).await
}

#[tauri::command]
pub async fn stop_connection(
    app: tauri::AppHandle,
    manager: State<'_, ConnectionManager>,
) -> Result<ConnectionSnapshot, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    manager.stop(&app, &dir).await
}

#[tauri::command]
pub async fn regenerate_tunnel(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    manager: State<'_, ConnectionManager>,
) -> Result<ConnectionSnapshot, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    manager.regenerate(&app, &dir, state.port).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn save_connection_settings(
    app: tauri::AppHandle,
    organization_id: Option<String>,
    workspace_id: Option<String>,
    tunnel_name: String,
    auto_start: bool,
    runtime_api_key: Option<String>,
    admin_api_key: Option<String>,
) -> Result<ConnectionSettingsSummary, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let mut loaded = settings::load(&dir)?;
    loaded.settings.organization_id = organization_id;
    loaded.settings.workspace_id = workspace_id;
    loaded.settings.tunnel_name = tunnel_name;
    loaded.settings.auto_start = auto_start;
    loaded.settings.normalize();

    let store = SecretStore::system();
    store.save(runtime_api_key.as_deref(), admin_api_key.as_deref())?;
    settings::save(&dir, &loaded.settings)?;
    Ok(settings_summary(loaded.settings, store.summary()?))
}

#[tauri::command]
pub fn connection_settings_summary(
    app: tauri::AppHandle,
) -> Result<ConnectionSettingsSummary, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let loaded = settings::load(&dir)?;
    let secret_summary = SecretStore::system().summary()?;
    Ok(settings_summary(loaded.settings, secret_summary))
}
