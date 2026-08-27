//! Tauri IPC commands backing the manager UI.
//!
//! Workspace/security administrative mutations record best-effort UI audit
//! rows after the backend mutation succeeds. This is intentionally distinct
//! from MCP tool auditing, which is fail-closed before a tool response is
//! returned. Commands use stable JSON errors where applicable.

use serde::Serialize;
use tauri::{Manager, State};

use crate::{
    browser::runtime::RuntimeProvisionState,
    connection::{
        manager::{ConnectionError, ConnectionManager as TailscaleManager},
        state::ConnectionState,
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
pub async fn set_paused(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    paused: bool,
) -> Result<(), String> {
    state.core.pause.set(paused);
    if let Err(e) = state.browser.on_pause_changed(paused).await {
        eprintln!("browser shutdown on Pause failed: {e}");
    }
    audit_ui(
        &state.core.db,
        None,
        if paused { "pause" } else { "resume" },
        None,
        None,
        None,
    );
    use tauri::Emitter;
    app.emit("access-paused", paused)
        .map_err(|e| e.to_string())?;
    // Keep the tray Pause/Resume label + status line in sync with the UI.
    crate::tray::update_paused(&app);
    Ok(())
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

/// The Tailscale state directory path, shown (elided) on the Settings page.
#[tauri::command]
pub async fn tailscale_state_dir(manager: State<'_, TailscaleManager>) -> Result<String, String> {
    Ok(manager.state_dir().await.to_string_lossy().to_string())
}

/// Redacted diagnostics report for the Settings page. Security invariants:
/// never contains the capability token, the full capability endpoint URL
/// (only its host FQDN), any Tailscale login auth URL, or file contents;
/// `recent_sidecar_logs` lines are pre-redacted by sidecar.rs.
#[derive(Serialize)]
pub struct DiagnosticsReport {
    pub app_version: &'static str,
    pub connection_state: String,
    pub local_port: u16,
    pub fqdn: Option<String>,
    pub paused: bool,
    pub workspace_count: usize,
    pub recent_sidecar_logs: Vec<String>,
}

/// FQDN only from states where it is present; never an endpoint URL.
fn safe_fqdn(state: &ConnectionState) -> Option<String> {
    match state {
        ConnectionState::Online { fqdn, .. } | ConnectionState::Verifying { fqdn } => {
            Some(fqdn.clone())
        }
        ConnectionState::Degraded { fqdn, .. } => fqdn.clone(),
        _ => None,
    }
}

/// Assembles the redacted diagnostics report.
#[tauri::command]
pub async fn connection_diagnostics(
    manager: State<'_, TailscaleManager>,
    state: State<'_, AppState>,
) -> Result<DiagnosticsReport, String> {
    let status = manager.status().await;
    let workspace_count = workspace_repo::list_workspaces(&state.core.db)
        .map_err(err_string)?
        .len();
    Ok(DiagnosticsReport {
        app_version: env!("CARGO_PKG_VERSION"),
        connection_state: status.state.state_tag().to_string(),
        local_port: status.local_port,
        fqdn: safe_fqdn(&status.state),
        paused: status.paused,
        workspace_count,
        recent_sidecar_logs: manager.diagnostics_snapshot().await,
    })
}

#[tauri::command]
pub fn get_app_settings(app: tauri::AppHandle) -> Result<settings::AppSettings, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    settings::load_app_settings(&dir).map_err(|e| settings_err("SETTINGS_INVALID", e))
}

/// Validates then persists application settings; rejects an invalid
/// `tailscale_hostname` before touching disk. After saving, runtime behavior
/// that follows these settings is synced (launch-at-login registration,
/// close-to-tray flag).
#[tauri::command]
pub async fn update_app_settings(
    app: tauri::AppHandle,
    settings: settings::AppSettings,
) -> Result<settings::AppSettings, String> {
    validate_hostname(&settings.tailscale_hostname)?;
    let state = app.state::<AppState>();
    state
        .agent_settings
        .save_full(&settings)
        .map_err(|e| settings_err("SETTINGS_SAVE_FAILED", e.message))?;
    state
        .browser
        .apply_settings(crate::browser::types::BrowserRuntimeSettings::from(
            &settings,
        ))
        .await
        .map_err(err_string)?;

    sync_launch_on_startup(&app, settings.launch_on_startup)?;
    use std::sync::atomic::Ordering;
    app.state::<crate::UiFlags>()
        .close_to_tray
        .store(settings.close_to_tray, Ordering::Relaxed);
    // The shared settings controller already synchronized Agent policy after
    // persistence. Unrestricted execution remains a separate desktop-only flag.
    state.core.unrestricted.set(settings.unrestricted_access);
    Ok(settings)
}

/// Aligns the OS launch-at-login registration with the saved preference.
fn sync_launch_on_startup(app: &tauri::AppHandle, want: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt as _;
    let auto = app.autolaunch();
    let have = auto
        .is_enabled()
        .map_err(|e| settings_err("AUTOSTART_FAILED", e))?;
    if want && !have {
        auto.enable()
            .map_err(|e| settings_err("AUTOSTART_FAILED", e))
    } else if !want && have {
        auto.disable()
            .map_err(|e| settings_err("AUTOSTART_FAILED", e))
    } else {
        Ok(())
    }
}

/// Truthful capability flags for the Lightpanda runtime: semantic automation
/// over native MCP works; visual/rendering features fail closed.
#[derive(Serialize)]
pub struct BrowserRuntimeCapabilities {
    pub visual_rendering: bool,
    pub screenshots: bool,
    pub file_upload: bool,
    pub managed_downloads: bool,
    pub native_mcp: bool,
}

/// Redacted browser runtime diagnostics for the Settings page (mirrors the
/// `BrowserRuntimeStatus` TypeScript type). Security invariants: never
/// contains an active page URL/title, history, cookies, download filenames,
/// or absolute host paths; only provision state, the pinned Lightpanda
/// version, and boolean flags.
#[derive(Serialize)]
pub struct BrowserRuntimeStatus {
    pub enabled: bool,
    pub running: bool,
    pub backend: &'static str,
    pub mode: &'static str,
    pub runtime_state: RuntimeProvisionState,
    pub lightpanda_version: Option<String>,
    pub wsl_installed: bool,
    pub distro_ready: bool,
    pub reboot_required: bool,
    pub repair_required: bool,
    pub capabilities: BrowserRuntimeCapabilities,
}

/// Boolean provision flags derived purely from the persisted runtime state.
/// Never spawns a process, so opening Settings stays side-effect free.
struct ProvisionFlags {
    wsl_installed: bool,
    distro_ready: bool,
    reboot_required: bool,
    repair_required: bool,
}

fn provision_flags(state: RuntimeProvisionState) -> ProvisionFlags {
    use RuntimeProvisionState as S;
    ProvisionFlags {
        wsl_installed: matches!(
            state,
            S::RebootRequired
                | S::DistroInstallRequired
                | S::InstallingDistro
                | S::LightpandaInstallRequired
                | S::InstallingLightpanda
                | S::VerifyingRuntime
                | S::Ready
                | S::RepairRequired
        ),
        distro_ready: state == S::Ready,
        reboot_required: state == S::RebootRequired,
        repair_required: state == S::RepairRequired,
    }
}

/// Assembles the redacted runtime status from the manager session state and
/// the persisted provisioner state. The manager's richer session status
/// (which includes the active page URL/title) is intentionally narrowed here.
async fn build_browser_runtime_status(state: &AppState) -> BrowserRuntimeStatus {
    let session = state.browser.status().await;
    let snapshot = state.browser_runtime_manager.state_snapshot();
    let runtime_state = snapshot
        .as_ref()
        .map(|s| s.state)
        .unwrap_or(RuntimeProvisionState::Uninitialized);
    let lightpanda_version = snapshot
        .as_ref()
        .and_then(|s| s.lightpanda_version.clone())
        .unwrap_or_else(|| {
            state
                .browser_runtime_manager
                .manifest()
                .lightpanda_version
                .clone()
        });
    let flags = provision_flags(runtime_state);
    BrowserRuntimeStatus {
        enabled: session.enabled,
        running: session.running,
        backend: "lightpanda",
        mode: "background",
        runtime_state,
        lightpanda_version: Some(lightpanda_version),
        wsl_installed: flags.wsl_installed,
        distro_ready: flags.distro_ready,
        reboot_required: flags.reboot_required,
        repair_required: flags.repair_required,
        capabilities: BrowserRuntimeCapabilities {
            visual_rendering: false,
            screenshots: false,
            file_upload: false,
            managed_downloads: false,
            native_mcp: true,
        },
    }
}

/// Redacted browser runtime diagnostics. Reads only in-memory and persisted
/// provisioner state; never starts downloads, provisioning, or a browser.
#[tauri::command]
pub async fn browser_runtime_status(
    state: State<'_, AppState>,
) -> Result<BrowserRuntimeStatus, String> {
    Ok(build_browser_runtime_status(&state).await)
}

/// Desktop-only runtime setup/resume: runs the idempotent provisioner and
/// returns fresh redacted status. Never exposed as an MCP tool.
#[tauri::command]
pub async fn setup_browser_runtime(
    state: State<'_, AppState>,
) -> Result<BrowserRuntimeStatus, String> {
    state
        .browser_runtime_manager
        .ensure_runtime()
        .await
        .map_err(err_string)?;
    audit_ui(
        &state.core.db,
        None,
        "setup_browser_runtime",
        None,
        None,
        None,
    );
    Ok(build_browser_runtime_status(&state).await)
}

/// Desktop-only runtime repair: revalidates and replaces only app-owned
/// runtime files, then returns fresh redacted status.
#[tauri::command]
pub async fn repair_browser_runtime(
    state: State<'_, AppState>,
) -> Result<BrowserRuntimeStatus, String> {
    state
        .browser_runtime_manager
        .repair_runtime()
        .await
        .map_err(err_string)?;
    audit_ui(
        &state.core.db,
        None,
        "repair_browser_runtime",
        None,
        None,
        None,
    );
    Ok(build_browser_runtime_status(&state).await)
}

/// Desktop-only Reset Browser Data: closes all page children, then removes
/// only the owned WSL profile/cookies/cache/artifacts. Never removes the
/// distro or the Lightpanda binary.
#[tauri::command]
pub async fn reset_browser_data(
    state: State<'_, AppState>,
) -> Result<BrowserRuntimeStatus, String> {
    state.browser.close().await.map_err(err_string)?;
    state
        .browser_runtime_manager
        .reset_browser_data()
        .await
        .map_err(err_string)?;
    audit_ui(&state.core.db, None, "reset_browser_data", None, None, None);
    Ok(build_browser_runtime_status(&state).await)
}

/// Desktop-only Remove Browser Runtime: closes all page children, then
/// unregisters the owned distro and deletes app-owned runtime directories
/// only after full ownership verification. The Browser Automation toggle is
/// preserved; the next browser use provisions again.
#[tauri::command]
pub async fn remove_browser_runtime(
    state: State<'_, AppState>,
) -> Result<BrowserRuntimeStatus, String> {
    state.browser.close().await.map_err(err_string)?;
    state
        .browser_runtime_manager
        .remove_runtime()
        .await
        .map_err(err_string)?;
    audit_ui(
        &state.core.db,
        None,
        "remove_browser_runtime",
        None,
        None,
        None,
    );
    Ok(build_browser_runtime_status(&state).await)
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

    #[test]
    fn provision_flags_track_persisted_state_without_side_effects() {
        let ready = provision_flags(RuntimeProvisionState::Ready);
        assert!(ready.wsl_installed);
        assert!(ready.distro_ready);
        assert!(!ready.reboot_required);
        assert!(!ready.repair_required);

        let reboot = provision_flags(RuntimeProvisionState::RebootRequired);
        assert!(reboot.wsl_installed);
        assert!(!reboot.distro_ready);
        assert!(reboot.reboot_required);

        let repair = provision_flags(RuntimeProvisionState::RepairRequired);
        assert!(repair.wsl_installed);
        assert!(!repair.distro_ready);
        assert!(repair.repair_required);

        for state in [
            RuntimeProvisionState::Uninitialized,
            RuntimeProvisionState::CheckingHost,
            RuntimeProvisionState::WslInstallRequired,
            RuntimeProvisionState::InstallingWsl,
            RuntimeProvisionState::UnsupportedHost,
        ] {
            let flags = provision_flags(state);
            assert!(!flags.wsl_installed, "{state:?}");
            assert!(!flags.distro_ready, "{state:?}");
            assert!(!flags.reboot_required, "{state:?}");
            assert!(!flags.repair_required, "{state:?}");
        }
    }

    #[test]
    fn runtime_status_serialization_is_redacted_and_fixed_shape() {
        let status = BrowserRuntimeStatus {
            enabled: true,
            running: true,
            backend: "lightpanda",
            mode: "background",
            runtime_state: RuntimeProvisionState::Ready,
            lightpanda_version: Some("0.3.7".into()),
            wsl_installed: true,
            distro_ready: true,
            reboot_required: false,
            repair_required: false,
            capabilities: BrowserRuntimeCapabilities {
                visual_rendering: false,
                screenshots: false,
                file_upload: false,
                managed_downloads: false,
                native_mcp: true,
            },
        };
        let json = serde_json::to_string(&status).unwrap();
        // Exact key whitelists prove no URL/title/history/cookie/path or
        // download-filename field can ever be serialized.
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let keys: std::collections::BTreeSet<String> =
            parsed.as_object().unwrap().keys().cloned().collect();
        let expected: std::collections::BTreeSet<String> = [
            "backend",
            "capabilities",
            "distro_ready",
            "enabled",
            "lightpanda_version",
            "mode",
            "reboot_required",
            "repair_required",
            "running",
            "runtime_state",
            "wsl_installed",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(keys, expected);
        let cap_keys: std::collections::BTreeSet<String> = parsed["capabilities"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let expected_caps: std::collections::BTreeSet<String> = [
            "file_upload",
            "managed_downloads",
            "native_mcp",
            "screenshots",
            "visual_rendering",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(cap_keys, expected_caps);
        for forbidden in [
            "url", "title", "history", "cookie", "filename", "c:\\", "/home/",
        ] {
            assert!(
                !json.to_ascii_lowercase().contains(forbidden),
                "runtime status leaks '{forbidden}': {json}"
            );
        }
    }
}
