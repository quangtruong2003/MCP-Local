mod commands;
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
            // Must be managed before any tunnel::spawn/stop/status access
            // (app.state::<TunnelHandle>() panics when unmanaged).
            app.manage(tunnel::TunnelHandle::default());
            println!("MCP server: http://127.0.0.1:{port}/mcp");
            // Restore persisted tunnel settings and auto-start the sidecar
            // if one was configured. Spawn failures are surfaced as a
            // tunnel-status Error event (inside spawn) rather than
            // blocking startup.
            let settings = settings::load(&dir).unwrap_or_default();
            if !settings.tunnel_id.is_empty() {
                let _ = tunnel::spawn(
                    app.handle(),
                    &settings.tunnel_id,
                    &settings.tunnel_api_key,
                    port,
                );
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
            commands::configure_tunnel,
            commands::stop_tunnel,
            commands::tunnel_status,
            commands::tunnel_config,
        ])
        .build(tauri::generate_context!())
        .expect("build tauri app")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                tunnel::stop(app_handle);
            }
        });
}
