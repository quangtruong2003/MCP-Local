//! In-process client -> real axum/rmcp stack against a temp workspace.

use rmcp::{model::CallToolRequestParams, transport::StreamableHttpClientTransport, ServiceExt};
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
            Ok(local_workspace_mcp_manager::server::WorkspaceServer::new(
                core_for_server.clone(),
            ))
        },
        std::sync::Arc::new(
            rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(),
        ),
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

/// Asserts exactly one audit row exists for `op` with the expected decision
/// and error_code, carrying the workspace id.
fn assert_audit_row(
    core: &lc_core::CoreServices,
    op: &str,
    ws_id: &str,
    expected_decision: &str,
    expected_error_code: Option<&str>,
) {
    let rows = lc_core::audit::list_recent(&core.db, 100).unwrap();
    let matching: Vec<_> = rows.iter().filter(|r| r.op == op).collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one '{op}' row, got {matching:?}"
    );
    let row = matching[0];
    assert_eq!(row.decision, expected_decision);
    assert_eq!(row.error_code.as_deref(), expected_error_code);
    assert_eq!(row.workspace_id.as_deref(), Some(ws_id));
    assert_eq!(row.source, "mcp");
}

#[tokio::test]
async fn full_read_write_roundtrip_over_mcp() {
    let (port, core, dir, ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://127.0.0.1:{port}/mcp"
        )))
        .await
        .unwrap();

    // list_directory sees the seeded file
    let res = client
        .call_tool(
            CallToolRequestParams::new("list_directory").with_arguments(
                serde_json::json!({ "workspace_id": ws_id, "path": "." })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert!(!res.is_error.unwrap_or(false));
    let text = serde_json::to_string(&res).unwrap();
    assert!(
        text.contains("hello.txt"),
        "listing missing hello.txt: {text}"
    );
    assert_audit_row(&core, "list_directory", &ws_id, "allow", None);

    // read_file returns a revision
    let res = client
        .call_tool(
            CallToolRequestParams::new("read_file").with_arguments(
                serde_json::json!({ "workspace_id": ws_id, "path": "hello.txt" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
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
    assert_audit_row(&core, "read_file", &ws_id, "allow", None);
    // Audit enrichment: relative path + safe byte metrics, never content.
    let row = latest_audit_row(&core, "read_file");
    assert_eq!(row.path.as_deref(), Some("hello.txt"));
    let detail = row.detail.as_ref().expect("read_file detail");
    assert!(detail["bytes"].as_u64().unwrap_or(0) > 0);
    assert!(detail["total_lines"].as_u64().unwrap_or(0) > 0);

    // apply_patch rewrites using that revision
    let res = client
        .call_tool(
            CallToolRequestParams::new("apply_patch").with_arguments(
                serde_json::json!({
                    "workspace_id": ws_id, "path": "hello.txt",
                    "old_string": "world", "new_string": "there",
                    "base_revision": rev
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
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
    assert_audit_row(&core, "apply_patch", &ws_id, "allow", None);
    // Audit enrichment: path plus exact byte delta ("world" -> "there": 0).
    let row = latest_audit_row(&core, "apply_patch");
    assert_eq!(row.path.as_deref(), Some("hello.txt"));
    let detail = row.detail.as_ref().expect("apply_patch detail");
    assert_eq!(detail["bytes_changed"].as_i64(), Some(0));
}

/// Returns the most recent audit row for `op` (panics if none).
fn latest_audit_row(core: &lc_core::CoreServices, op: &str) -> lc_core::models::AuditEvent {
    let rows = lc_core::audit::list_recent(&core.db, 100).unwrap();
    rows.into_iter()
        .find(|r| r.op == op)
        .unwrap_or_else(|| panic!("no audit row for '{op}'"))
}

#[tokio::test]
async fn audit_rows_record_denials() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://127.0.0.1:{port}/mcp"
        )))
        .await
        .unwrap();

    // Unknown workspace -> error row with NULL workspace_id.
    let res = client
        .call_tool(
            CallToolRequestParams::new("read_file").with_arguments(
                serde_json::json!({
                    "workspace_id": "ghost", "path": "x"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    assert!(res.is_error.unwrap_or(false));

    // Write on a read-only workspace -> deny row carrying the workspace id.
    lc_core::workspace_repo::update_capabilities(&core.db, &ws_id, true, false).unwrap();
    let res = client
        .call_tool(
            CallToolRequestParams::new("create_file").with_arguments(
                serde_json::json!({
                    "workspace_id": ws_id, "path": "new.txt", "content": "hi"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    assert!(res.is_error.unwrap_or(false));

    let rows = lc_core::audit::list_recent(&core.db, 100).unwrap();
    let by_op = |op: &str| rows.iter().filter(|r| r.op == op).collect::<Vec<_>>();

    let not_found = by_op("read_file");
    assert_eq!(not_found.len(), 1);
    assert_eq!(not_found[0].decision, "error");
    assert_eq!(
        not_found[0].error_code.as_deref(),
        Some("WORKSPACE_NOT_FOUND")
    );
    assert_eq!(not_found[0].workspace_id, None);

    let write_denied = by_op("create_file");
    assert_eq!(write_denied.len(), 1);
    assert_eq!(write_denied[0].decision, "deny");
    assert_eq!(
        write_denied[0].error_code.as_deref(),
        Some("WRITE_DISABLED")
    );
    assert_eq!(
        write_denied[0].workspace_id.as_deref(),
        Some(ws_id.as_str())
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
        .call_tool(
            CallToolRequestParams::new("list_directory").with_arguments(
                serde_json::json!({ "workspace_id": ws_id })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false), "paused call must be isError");
    assert!(
        text.contains("ACCESS_PAUSED"),
        "missing ACCESS_PAUSED: {text}"
    );
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
        .call_tool(
            CallToolRequestParams::new("read_file").with_arguments(
                serde_json::json!({ "workspace_id": "nope", "path": "x.txt" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false));
    assert!(text.contains("WORKSPACE_NOT_FOUND"), "{text}");
}

/// Scenario B: path traversal escapes are denied at the MCP layer with
/// PATH_OUTSIDE_WORKSPACE, and the deny row lands in the audit log.
#[tokio::test]
async fn path_traversal_denied_over_mcp() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://127.0.0.1:{port}/mcp"
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("read_file").with_arguments(
                serde_json::json!({
                    "workspace_id": ws_id, "path": "../../../outside.txt"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false), "traversal must be isError");
    assert!(text.contains("PATH_OUTSIDE_WORKSPACE"), "{text}");
    // Post-prelude failures are classified "error" with their code (pause and
    // capability checks are the "deny" rows) — established pipeline convention.
    assert_audit_row(&core, "read_file", &ws_id, "error", Some("PATH_OUTSIDE_WORKSPACE"));
}

/// Scenario C: sensitive files (.env) return SENSITIVE_FILE_BLOCKED.
#[tokio::test]
async fn sensitive_file_blocked_over_mcp() {
    let (port, core, dir, ws_id) = spawn_server().await;
    std::fs::write(dir.path().join("ws/.env"), "SECRET=1\n").unwrap();
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://127.0.0.1:{port}/mcp"
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("read_file").with_arguments(
                serde_json::json!({ "workspace_id": ws_id, "path": ".env" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false));
    assert!(text.contains("SENSITIVE_FILE_BLOCKED"), "{text}");
    // The secret must not leak into the error payload.
    assert!(!text.contains("SECRET"), "secret leaked: {text}");
    assert_audit_row(&core, "read_file", &ws_id, "error", Some("SENSITIVE_FILE_BLOCKED"));
    // The audit row must not leak the secret either: no file content in
    // path or detail (the blocked filename itself is allowed).
    let row = latest_audit_row(&core, "read_file");
    let detail_str = row
        .detail
        .as_ref()
        .map(|d| d.to_string())
        .unwrap_or_default();
    assert!(
        !detail_str.contains("SECRET") && !row.path.as_deref().unwrap_or("").contains("SECRET"),
        "audit leak: path={:?} detail={detail_str}",
        row.path
    );
}

/// Scenario E: an external edit between read and patch yields
/// FILE_CHANGED_SINCE_READ instead of silently overwriting.
#[tokio::test]
async fn stale_revision_detected_over_mcp() {
    let (port, core, dir, ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://127.0.0.1:{port}/mcp"
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("read_file").with_arguments(
                serde_json::json!({ "workspace_id": ws_id, "path": "hello.txt" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let body_text = res.content.first().and_then(|c| c.as_text().map(|t| t.text.to_string())).unwrap_or_default();
    let rev: String = extract_json(&body_text)["revision"].as_str().unwrap().into();

    // External edit behind ChatGPT's back.
    std::fs::write(dir.path().join("ws/hello.txt"), "externally edited\n").unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("apply_patch").with_arguments(
                serde_json::json!({
                    "workspace_id": ws_id, "path": "hello.txt",
                    "old_string": "world", "new_string": "there",
                    "base_revision": rev
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false));
    assert!(text.contains("FILE_CHANGED_SINCE_READ"), "{text}");
    // The external edit must be intact — no silent overwrite.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("ws/hello.txt")).unwrap(),
        "externally edited\n"
    );
    assert_audit_row(&core, "apply_patch", &ws_id, "error", Some("FILE_CHANGED_SINCE_READ"));
}

/// Scenario H: two workspaces — a path valid in workspace 1 is rejected when
/// passed with workspace 2's id (cross-confinement).
#[tokio::test]
async fn cross_workspace_confinement_over_mcp() {
    let (port, core, dir, _ws_a) = spawn_server().await;
    let db = &core.db;
    std::fs::create_dir_all(dir.path().join("ws-b")).unwrap();
    std::fs::write(dir.path().join("ws-b/secret-b.txt"), "b\n").unwrap();
    let ws_b = lc_core::workspace_repo::insert_workspace(
        db,
        "B",
        &dir.path().join("ws-b").to_string_lossy(),
    )
    .unwrap();

    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://127.0.0.1:{port}/mcp"
        )))
        .await
        .unwrap();

    // Absolute path into workspace A, presented under workspace B's id.
    let abs = dir.path().join("ws/hello.txt").display().to_string().replace('\\', "/");
    let res = client
        .call_tool(
            CallToolRequestParams::new("read_file").with_arguments(
                serde_json::json!({
                    "workspace_id": ws_b.id, "path": abs
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false));
    assert!(text.contains("PATH_OUTSIDE_WORKSPACE"), "{text}");
}
