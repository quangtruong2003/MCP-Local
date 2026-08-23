#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod server;

#[tokio::main]
async fn main() {
    // Transitional scaffold path; Task 12 wires the real app-data path.
    let db_path = std::env::temp_dir().join("local-workspace-mcp-manager-scaffold.db");
    let core = std::sync::Arc::new(
        lc_core::CoreServices::open(&db_path).expect("open scaffold database"),
    );
    let port = server::serve(core.clone()).await.expect("bind loopback");
    println!("mcp server on 127.0.0.1:{port}/mcp");
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("tauri run");
}
