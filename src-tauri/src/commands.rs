//! Tauri IPC commands backing the manager UI.
//!
//! Workspace/security administrative mutations record best-effort UI audit
//! rows after the backend mutation succeeds. This is intentionally distinct
//! from MCP tool auditing, which is fail-closed before a tool response is
//! returned. Commands use stable JSON errors where applicable.

use serde::Serialize;
use tauri::{Manager, State};

use crate::{
    browser::types::{BrowserEngine, BrowserRunMode},
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

/// Redacted browser runtime diagnostics for the Settings page (mirrors the
/// `BrowserRuntimeStatus` TypeScript type). Security invariants: never
/// contains an active page URL/title, history, cookies, or download
/// filenames; only availability flags and bundled runtime versions.
#[derive(Serialize)]
pub struct BrowserRuntimeStatus {
    pub enabled: bool,
    pub running: bool,
    pub configured_engine: BrowserEngine,
    pub run_mode: BrowserRunMode,
    pub bundled_chromium_available: bool,
    pub chrome_available: bool,
    pub edge_available: bool,
    pub firefox_available: bool,
    pub node_version: Option<String>,
    pub playwright_mcp_version: Option<String>,
    pub playwright_version: Option<String>,
}

/// Subset of `runtime-manifest.json` written by scripts/build-browser-runtime.ps1.
/// Fields are optional so a partial/older manifest degrades to "unknown".
#[derive(Debug, Default, serde::Deserialize)]
struct RuntimeManifest {
    #[serde(default)]
    node_version: Option<String>,
    #[serde(default)]
    playwright_mcp_version: Option<String>,
    #[serde(default)]
    playwright_version: Option<String>,
    #[serde(default)]
    browser_entries: Vec<RuntimeManifestBrowserEntry>,
}

#[derive(Debug, serde::Deserialize)]
struct RuntimeManifestBrowserEntry {
    name: String,
}

/// Reads the bundled runtime manifest; a missing or invalid file yields `None`
/// so diagnostics degrade gracefully instead of failing.
fn read_runtime_manifest(runtime_root: &std::path::Path) -> Option<RuntimeManifest> {
    let bytes = std::fs::read(runtime_root.join("runtime-manifest.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// A bundled browser is available only when the manifest lists an entry with
/// the expected prefix AND the matching `browsers/<name>` directory exists.
fn bundled_browser_available(
    manifest: Option<&RuntimeManifest>,
    runtime_root: &std::path::Path,
    entry_prefix: &str,
) -> bool {
    manifest.is_some_and(|manifest| {
        manifest.browser_entries.iter().any(|entry| {
            entry.name.starts_with(entry_prefix)
                && runtime_root.join("browsers").join(&entry.name).is_dir()
        })
    })
}

const CHROME_INSTALL_RELATIVE: &str = "Google/Chrome/Application/chrome.exe";
const EDGE_INSTALL_RELATIVE: &str = "Microsoft/Edge/Application/msedge.exe";

/// Candidate install locations for a system browser channel, derived from the
/// standard Windows roots. Pure function so tests can pin the roots.
fn channel_install_candidates(
    program_files: Option<&std::path::Path>,
    program_files_x86: Option<&std::path::Path>,
    local_app_data: Option<&std::path::Path>,
    relative: &str,
) -> Vec<std::path::PathBuf> {
    [program_files, program_files_x86, local_app_data]
        .into_iter()
        .flatten()
        .map(|root| root.join(relative))
        .collect()
}

/// Filesystem-only probe for a system browser channel. Never starts a page or
/// a browser process.
fn system_channel_available(relative: &str) -> bool {
    let program_files = std::env::var_os("ProgramFiles").map(std::path::PathBuf::from);
    let program_files_x86 = std::env::var_os("ProgramFiles(x86)").map(std::path::PathBuf::from);
    let local_app_data = std::env::var_os("LocalAppData").map(std::path::PathBuf::from);
    channel_install_candidates(
        program_files.as_deref(),
        program_files_x86.as_deref(),
        local_app_data.as_deref(),
        relative,
    )
    .iter()
    .any(|path| path.is_file())
}

/// Assembles the redacted runtime status from the manager session state and
/// the bundled runtime manifest. The manager's richer session status (which
/// includes the active page URL/title) is intentionally narrowed here.
async fn build_browser_runtime_status(state: &AppState) -> BrowserRuntimeStatus {
    let session = state.browser.status().await;
    let manifest = read_runtime_manifest(&state.browser_runtime);
    BrowserRuntimeStatus {
        enabled: session.enabled,
        running: session.running,
        configured_engine: session.configured_engine,
        run_mode: session.run_mode,
        bundled_chromium_available: bundled_browser_available(
            manifest.as_ref(),
            &state.browser_runtime,
            "chromium-",
        ),
        chrome_available: system_channel_available(CHROME_INSTALL_RELATIVE),
        edge_available: system_channel_available(EDGE_INSTALL_RELATIVE),
        firefox_available: bundled_browser_available(
            manifest.as_ref(),
            &state.browser_runtime,
            "firefox-",
        ),
        node_version: manifest.as_ref().and_then(|m| m.node_version.clone()),
        playwright_mcp_version: manifest
            .as_ref()
            .and_then(|m| m.playwright_mcp_version.clone()),
        playwright_version: manifest.as_ref().and_then(|m| m.playwright_version.clone()),
    }
}

/// Redacted browser runtime diagnostics: reads the runtime manifest and probes
/// Chrome/Edge channel availability without starting a page.
#[tauri::command]
pub async fn browser_runtime_status(
    state: State<'_, AppState>,
) -> Result<BrowserRuntimeStatus, String> {
    Ok(build_browser_runtime_status(&state).await)
}

/// Desktop-only profile reset. Accepts the engine enum (never a filesystem
/// path), stops any active browser first, then wipes and recreates exactly
/// `<app-data>/browser/profiles/<engine>/` via the manager's canonicalized
/// confinement check. Records a `source=ui` audit row and returns fresh
/// runtime status.
#[tauri::command]
pub async fn reset_browser_profile(
    engine: BrowserEngine,
    state: State<'_, AppState>,
) -> Result<BrowserRuntimeStatus, String> {
    state
        .browser
        .reset_profile(engine)
        .await
        .map_err(err_string)?;
    // Audit only the managed-relative profile path, never the absolute
    // filesystem location (final-review Minor 3).
    let audit_path = format!(
        "browser/profiles/{}",
        crate::browser::manager::BrowserManager::engine_slug(engine)
    );
    audit_ui(
        &state.core.db,
        None,
        "reset_browser_profile",
        None,
        Some(&audit_path),
        Some(serde_json::json!({ "engine": engine })),
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

    fn write_manifest(dir: &std::path::Path, json: &str) {
        std::fs::write(dir.join("runtime-manifest.json"), json).unwrap();
    }

    #[test]
    fn runtime_manifest_versions_and_entries_parse() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            r#"{
                "node_version": "24.19.0",
                "playwright_mcp_version": "0.0.79",
                "playwright_version": "1.57.0",
                "platform": "windows-x64",
                "browser_entries": [{"name": "chromium-1200"}, {"name": "firefox-1475"}]
            }"#,
        );
        let manifest = read_runtime_manifest(dir.path()).unwrap();
        assert_eq!(manifest.node_version.as_deref(), Some("24.19.0"));
        assert_eq!(manifest.playwright_mcp_version.as_deref(), Some("0.0.79"));
        assert_eq!(manifest.playwright_version.as_deref(), Some("1.57.0"));
        assert_eq!(manifest.browser_entries.len(), 2);
    }

    #[test]
    fn missing_or_invalid_manifest_degrades_to_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_runtime_manifest(dir.path()).is_none());
        write_manifest(dir.path(), "{not json");
        assert!(read_runtime_manifest(dir.path()).is_none());
    }

    #[test]
    fn bundled_availability_requires_manifest_entry_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            r#"{"browser_entries": [{"name": "chromium-1200"}]}"#,
        );
        let manifest = read_runtime_manifest(dir.path()).unwrap();
        // Entry listed but the browser directory is missing -> not available.
        assert!(!bundled_browser_available(
            Some(&manifest),
            dir.path(),
            "chromium-"
        ));
        // No manifest at all -> not available.
        assert!(!bundled_browser_available(None, dir.path(), "chromium-"));
        // Entry plus matching directory -> available; other prefixes stay false.
        std::fs::create_dir_all(dir.path().join("browsers").join("chromium-1200")).unwrap();
        assert!(bundled_browser_available(
            Some(&manifest),
            dir.path(),
            "chromium-"
        ));
        assert!(!bundled_browser_available(
            Some(&manifest),
            dir.path(),
            "firefox-"
        ));
    }

    #[test]
    fn channel_candidates_cover_standard_windows_locations() {
        use std::path::Path;
        let candidates = channel_install_candidates(
            Some(Path::new("C:/Program Files")),
            Some(Path::new("C:/Program Files (x86)")),
            Some(Path::new("C:/Users/u/AppData/Local")),
            CHROME_INSTALL_RELATIVE,
        );
        assert!(candidates
            .iter()
            .any(|p| p.ends_with("Google/Chrome/Application/chrome.exe")));
        assert_eq!(candidates.len(), 3);

        let edge = channel_install_candidates(
            Some(Path::new("C:/Program Files")),
            None,
            None,
            EDGE_INSTALL_RELATIVE,
        );
        assert_eq!(edge.len(), 1);
        assert!(edge[0].ends_with("Microsoft/Edge/Application/msedge.exe"));
    }

    #[test]
    fn runtime_status_json_never_carries_page_or_file_fields() {
        let status = BrowserRuntimeStatus {
            enabled: true,
            running: true,
            configured_engine: BrowserEngine::BundledChromium,
            run_mode: BrowserRunMode::Background,
            bundled_chromium_available: true,
            chrome_available: false,
            edge_available: false,
            firefox_available: true,
            node_version: Some("24.19.0".into()),
            playwright_mcp_version: Some("0.0.79".into()),
            playwright_version: Some("1.57.0".into()),
        };
        let json = serde_json::to_string(&status).unwrap();
        for forbidden in ["url", "title", "history", "cookie", "filename", "download"] {
            assert!(
                !json.to_ascii_lowercase().contains(forbidden),
                "runtime status leaks '{forbidden}': {json}"
            );
        }
    }
}
