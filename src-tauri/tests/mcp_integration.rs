//! In-process client -> real axum/rmcp stack against a temp workspace.

use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::StreamableHttpClientTransport,
};
use std::sync::Arc;

type Core = Arc<lc_core::CoreServices>;

async fn spawn_server() -> (u16, Core, tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let db = lc_core::db::Database::open(&dir.path().join("t.db")).unwrap();
    std::fs::create_dir_all(dir.path().join("ws")).unwrap();
    std::fs::write(dir.path().join("ws/hello.txt"), "hello world\n").unwrap();
    let ws = lc_core::workspace_repo::insert_workspace(
        &db,
        "W",
        &dir.path().join("ws").to_string_lossy(),
    )
    .unwrap();
    // Workspaces are created read-only; the roundtrip test needs write access.
    lc_core::workspace_repo::update_capabilities(&db, &ws.id, true, true).unwrap();
    let core = Arc::new(lc_core::CoreServices {
        db,
        pause: Default::default(),
        sensitive: lc_core::sensitive::SensitiveMatcher::new(),
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let core_for_server = core.clone();
    let service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        move || {
            Ok(
                local_workspace_mcp_manager::server::WorkspaceServer::new(
                    core_for_server.clone(),
                ),
            )
        },
        std::sync::Arc::new(rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default()),
        Default::default(),
    );
    let app = axum::Router::new().nest_service("/mcp", service);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (port, core, dir, ws.id)
}

/// Extract the first JSON object/array embedded in a serialized response
/// (content may be wrapped in SSE frames in some versions).
fn extract_json(text: &str) -> serde_json::Value {
    let s = text.trim();
    let start = s.find(['{', '[']).unwrap();
    let end = s.rfind(['}', ']']).unwrap();
    serde_json::from_str(&s[start..=end]).unwrap()
}

#[tokio::test]
async fn full_read_write_roundtrip_over_mcp() {
    let (port, _core, dir, ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://127.0.0.1:{port}/mcp"
        )))
        .await
        .unwrap();

    // list_directory sees the seeded file
    let res = client
        .call_tool(CallToolRequestParams::new("list_directory").with_arguments(serde_json::json!({ "workspace_id": ws_id, "path": "." })
                    .as_object().unwrap().clone()))
        .await
        .unwrap();
    assert!(!res.is_error.unwrap_or(false));
    let text = serde_json::to_string(&res).unwrap();
    assert!(text.contains("hello.txt"), "listing missing hello.txt: {text}");

    // read_file returns a revision
    let res = client
        .call_tool(CallToolRequestParams::new("read_file").with_arguments(serde_json::json!({ "workspace_id": ws_id, "path": "hello.txt" })
                    .as_object().unwrap().clone()))
        .await
        .unwrap();
    // The revision lives inside the tool's JSON payload in content[0].text.
    let body_text = res
        .content
        .first()
        .and_then(|c| c.as_text().map(|t| t.text.to_string()))
        .unwrap_or_default();
    let rev: String = extract_json(&body_text)["revision"]
        .as_str()
        .unwrap()
        .into();

    // apply_patch rewrites using that revision
    let res = client
        .call_tool(CallToolRequestParams::new("apply_patch").with_arguments(serde_json::json!({
                    "workspace_id": ws_id, "path": "hello.txt",
                    "old_string": "world", "new_string": "there",
                    "base_revision": rev
                })
                .as_object().unwrap().clone()))
        .await
        .unwrap();
    assert!(
        !res.is_error.unwrap_or(false),
        "patch failed: {}",
        serde_json::to_string(&res).unwrap()
    );
    // PatchResult carries {path, revision, bytes_changed}; verify the write
    // by reading the file back from disk.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("ws/hello.txt")).unwrap(),
        "hello there\n"
    );
}

#[tokio::test]
async fn pause_blocks_tool_calls() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://127.0.0.1:{port}/mcp"
        )))
        .await
        .unwrap();

    core.pause.set(true);
    let res = client
        .call_tool(CallToolRequestParams::new("list_directory").with_arguments(serde_json::json!({ "workspace_id": ws_id })
                    .as_object().unwrap().clone()))
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false), "paused call must be isError");
    assert!(text.contains("ACCESS_PAUSED"), "missing ACCESS_PAUSED: {text}");
}

#[tokio::test]
async fn unknown_workspace_is_invalid_error_result() {
    let (port, _core, _dir, _ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://127.0.0.1:{port}/mcp"
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(CallToolRequestParams::new("read_file").with_arguments(serde_json::json!({ "workspace_id": "nope", "path": "x.txt" })
                    .as_object().unwrap().clone()))
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false));
    assert!(text.contains("WORKSPACE_NOT_FOUND"), "{text}");
}
