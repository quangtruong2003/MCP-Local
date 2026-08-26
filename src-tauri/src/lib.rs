mod agent_skills;
pub mod browser;
mod commands;
pub mod connection;
mod execution;
mod process_job;
pub mod screen_capture;
pub mod secret_store;
pub mod server;
pub mod settings;
pub mod tray;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::Manager;

pub struct AppState {
    pub core: Arc<lc_core::CoreServices>,
    pub port: u16,
    pub agent_policy: Arc<settings::AgentPolicyState>,
    pub agent_settings: Arc<settings::AgentSettingsController>,
    pub browser: Arc<browser::manager::BrowserManager>,
}

/// Runtime UI behavior flags mirrored from [`settings::AppSettings`] so the
/// window-close handler can read them without touching disk.
pub struct UiFlags {
    pub close_to_tray: AtomicBool,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Single instance: a second launch focuses the running window instead
        // of failing to bind the same MCP port.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let core = Arc::new(
                lc_core::CoreServices::open(&dir.join("manager.db")).map_err(|e| e.to_string())?,
            );
            let audit_app = app.handle().clone();
            core.db
                .set_audit_listener(Arc::new(move |event: &lc_core::models::AuditEvent| {
                    use tauri::Emitter;
                    if let Err(error) = audit_app.emit("audit-recorded", event) {
                        eprintln!("audit event emit failed: {error}");
                    }
                }));
            // One-time migration from the MVP plaintext tunnel_api_key. Only
            // rewrite settings.json after the OS credential write succeeds.
            let loaded = settings::load(&dir).unwrap_or_else(|e| {
                eprintln!("settings load failed: {e}");
                settings::LoadedSettings {
                    settings: settings::ConnectionSettings::default(),
                    legacy_runtime_key: None,
                }
            });
            if let Some(legacy_key) = loaded.legacy_runtime_key.as_deref() {
                match secret_store::SecretStore::system().save(Some(legacy_key), None) {
                    Ok(()) => {
                        if let Err(e) = settings::save(&dir, &loaded.settings) {
                            eprintln!("failed to remove legacy plaintext key: {e}");
                        }
                    }
                    Err(e) => eprintln!("legacy key migration failed: {e}"),
                }
            }

            // Load the current application policy before exposing the MCP
            // endpoint so the very first request sees the persisted Agent and
            // Browser Automation settings rather than transient defaults.
            let app_settings = settings::load_app_settings(&dir).unwrap_or_else(|e| {
                eprintln!("app settings load failed: {e}");
                settings::AppSettings::default()
            });
            let agent_policy = Arc::new(settings::AgentPolicyState::from_settings(&app_settings));
            let agent_settings = Arc::new(settings::AgentSettingsController::new(
                dir.clone(),
                agent_policy.clone(),
            ));

            let resource_runtime = app.path().resource_dir()?.join("browser-runtime");
            let development_runtime =
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("browser-runtime");
            let browser_runtime = if resource_runtime.join("runtime-manifest.json").is_file() {
                resource_runtime
            } else {
                development_runtime
            };
            let browser = Arc::new(
                browser::manager::BrowserManager::with_playwright(
                    core.clone(),
                    dir.clone(),
                    browser_runtime,
                    &app_settings,
                )
                .map_err(|e| e.to_string())?,
            );

            let port = tauri::async_runtime::block_on(server::serve(
                core.clone(),
                agent_policy.clone(),
                agent_settings.clone(),
                browser.clone(),
            ))?;
            app.manage(AppState {
                core: core.clone(),
                port,
                agent_policy: agent_policy.clone(),
                agent_settings,
                browser,
            });
            println!("MCP server: http://127.0.0.1:{port}/mcp");

            app.manage(UiFlags {
                close_to_tray: AtomicBool::new(app_settings.close_to_tray),
            });
            // Mirror the persisted unrestricted-access preference into the
            // live pipeline flag (default off).
            core.unrestricted.set(app_settings.unrestricted_access);

            // Self-heal the autostart registration to match the persisted
            // preference (covers registry edits and setting races).
            sync_autostart(app.handle(), app_settings.launch_on_startup);

            let tailscale_manager = connection::manager::ConnectionManager::with_parts(
                std::sync::Arc::new(connection::manager::ProcessSidecarDriver),
                std::sync::Arc::new(connection::manager::ReqwestHealthVerifier),
                connection::manager::ConnectionManager::tauri_emitter(app.handle().clone()),
                connection::secrets::CapabilitySecretStore::new(dir.clone()),
                dir.join("tailscale"),
                app_settings.tailscale_hostname.clone(),
                port,
            );
            app.manage(tailscale_manager);

            tray::build(app.handle()).map_err(|e| format!("tray init failed: {e}"))?;

            // The window is created hidden (config `visible: false`) so a
            // minimized-to-tray launch never flashes it. Show it unless the
            // user opted to start in the tray; the tray menu and the
            // single-instance handler reveal it on demand.
            if !app_settings.start_minimized_to_tray {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }

            if app_settings.start_on_launch {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let manager = handle.state::<connection::manager::ConnectionManager>();
                    let _ = manager.start().await;
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_workspace,
            commands::list_workspaces,
            commands::set_capabilities,
            commands::set_execution_access,
            commands::remove_workspace,
            commands::set_paused,
            commands::recent_audit,
            commands::server_status,
            commands::connection_status,
            commands::start_connection,
            commands::stop_connection,
            commands::retry_connection,
            commands::rotate_connection_url,
            commands::reset_tailscale_identity,
            commands::tailscale_state_dir,
            commands::connection_diagnostics,
            commands::get_app_settings,
            commands::update_app_settings,
        ])
        .build(tauri::generate_context!())
        .expect("build tauri app")
        .run(|app_handle, event| match event {
            // Close-to-tray: hide instead of exiting when the user closes the
            // main window and the setting allows it. Quit stays available via
            // the tray menu.
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" => {
                let hide = app_handle
                    .state::<UiFlags>()
                    .close_to_tray
                    .load(Ordering::Relaxed);
                if hide {
                    api.prevent_close();
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.hide();
                    }
                }
            }
            tauri::RunEvent::ExitRequested { .. } => {
                // Gracefully stop both managed child-process trees. Their Job
                // Objects remain the orphan-safety net if teardown interrupts
                // either async stop.
                let manager = app_handle.state::<connection::manager::ConnectionManager>();
                if let Err(e) = tauri::async_runtime::block_on(manager.stop()) {
                    eprintln!("graceful connection stop on exit failed: {}", e.message);
                }
                let browser = app_handle.state::<AppState>().browser.clone();
                if let Err(e) = tauri::async_runtime::block_on(browser.on_pause_changed(true)) {
                    eprintln!("graceful browser stop on exit failed: {e}");
                }
            }
            _ => {}
        });
}

/// Enables/disables the OS-level launch-at-login registration so it matches
/// `launch_on_startup`. Failures are logged, not fatal (the stored setting
/// remains the source of truth for the next attempt).
fn sync_autostart(app: &tauri::AppHandle, want: bool) {
    use tauri_plugin_autostart::ManagerExt as _;
    let auto = app.autolaunch();
    let have = auto.is_enabled().unwrap_or(false);
    let result = if want && !have {
        auto.enable()
    } else if !want && have {
        auto.disable()
    } else {
        return;
    };
    if let Err(e) = result {
        eprintln!("autostart sync failed: {e}");
    }
}
