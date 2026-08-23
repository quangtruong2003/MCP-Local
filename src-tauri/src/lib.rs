mod commands;
pub mod server;
pub mod settings;

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
            println!("MCP server: http://127.0.0.1:{port}/mcp");
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
