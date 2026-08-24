mod commands;
pub mod connection;
mod execution;
mod process_job;
pub mod secret_store;
pub mod server;
pub mod settings;
pub mod tunnel;

use std::sync::Arc;
use tauri::Manager;

pub struct AppState {
    pub core: Arc<lc_core::CoreServices>,
    pub port: u16,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let core = Arc::new(
                lc_core::CoreServices::open(&dir.join("manager.db")).map_err(|e| e.to_string())?,
            );
            let port = tauri::async_runtime::block_on(server::serve(core.clone()))?;
            app.manage(AppState { core, port });
            app.manage(tunnel::TunnelHandle::default());
            println!("MCP server: http://127.0.0.1:{port}/mcp");

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

            // Versioned v2 settings drive the Tailscale connection manager.
            let app_settings = settings::load_app_settings(&dir).unwrap_or_else(|e| {
                eprintln!("app settings load failed: {e}");
                settings::AppSettings::default()
            });
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
            commands::get_app_settings,
            commands::update_app_settings,
        ])
        .build(tauri::generate_context!())
        .expect("build tauri app")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                tunnel::stop(app_handle);
            }
        });
}
