#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod server;

#[tokio::main]
async fn main() {
    let core = std::sync::Arc::new(lc_core::CoreServices);
    let port = server::serve(core.clone()).await.expect("bind loopback");
    println!("mcp server on 127.0.0.1:{port}/mcp");
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("tauri run");
}
