pub mod server;

/// Transitional scaffold entry point; Task 12 wires the real app-data path.
pub fn run() {
    let db_path = std::env::temp_dir().join("local-workspace-mcp-manager-scaffold.db");
    let core = std::sync::Arc::new(
        lc_core::CoreServices::open(&db_path).expect("open scaffold database"),
    );
    let port = tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(server::serve(core))
        .expect("bind loopback");
    println!("mcp server on 127.0.0.1:{port}/mcp");
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("tauri run");
}
