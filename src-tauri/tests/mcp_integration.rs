//! In-process client -> real axum/rmcp stack against a temp workspace.

use base64::Engine as _;
use rmcp::{
    model::{CallToolRequestParams, GetPromptRequestParams},
    transport::StreamableHttpClientTransport,
    ServiceExt,
};
use std::{io::Cursor, sync::Arc};

use local_workspace_mcp_manager::screen_capture::{
    CaptureGeometry, CaptureTarget, CaptureTargetKind, CapturedImage, ScreenCaptureProvider,
};

type Core = Arc<lc_core::CoreServices>;

#[derive(Debug, Default)]
struct FakeCaptureProvider;

fn fake_display_target() -> CaptureTarget {
    CaptureTarget {
        id: "display:1".into(),
        kind: CaptureTargetKind::Display,
        name: "Display 1".into(),
        application: None,
        geometry: CaptureGeometry {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        },
        scale_factor: 1.0,
        primary: true,
    }
}

fn fake_window_target() -> CaptureTarget {
    CaptureTarget {
        id: "window:7".into(),
        kind: CaptureTargetKind::Window,
        name: "Editor".into(),
        application: Some("TestApp".into()),
        geometry: CaptureGeometry {
            x: 40,
            y: 50,
            width: 800,
            height: 600,
        },
        scale_factor: 1.0,
        primary: false,
    }
}

fn fake_png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([1, 2, 3, 255]));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

impl ScreenCaptureProvider for FakeCaptureProvider {
    fn list_targets(&self) -> Result<Vec<CaptureTarget>, lc_core::error::CoreError> {
        Ok(vec![fake_display_target(), fake_window_target()])
    }

    fn capture(
        &self,
        target_id: &str,
        _max_edge: Option<u32>,
    ) -> Result<CapturedImage, lc_core::error::CoreError> {
        let target = match target_id {
            "display:1" => fake_display_target(),
            "window:7" => fake_window_target(),
            _ => {
                return Err(lc_core::error::CoreError::new(
                    lc_core::error::ErrorCode::CaptureTargetNotFound,
                    "missing capture target",
                ))
            }
        };
        Ok(CapturedImage {
            png_bytes: fake_png(),
            target,
            original_width: 2,
            original_height: 1,
            returned_width: 2,
            returned_height: 1,
        })
    }
}

/// Internal transport used only by this in-process MCP server test suite.
/// Production clients never connect to this URL directly: Tailscale Funnel
/// terminates public HTTPS and reverse-proxies the protected capability path
/// to the same loopback `/mcp` service.
fn local_test_mcp_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

async fn spawn_server() -> (u16, Core, tempfile::TempDir, String) {
    spawn_server_with_settings(local_workspace_mcp_manager::settings::AppSettings::default()).await
}

async fn spawn_server_with_settings(
    settings: local_workspace_mcp_manager::settings::AppSettings,
) -> (u16, Core, tempfile::TempDir, String) {
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
        unrestricted: Default::default(),
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let core_for_server = core.clone();
    local_workspace_mcp_manager::settings::save_app_settings(dir.path(), &settings).unwrap();
    let agent_policy =
        Arc::new(local_workspace_mcp_manager::settings::AgentPolicyState::from_settings(&settings));
    let agent_policy_for_server = agent_policy.clone();
    let agent_settings = Arc::new(
        local_workspace_mcp_manager::settings::AgentSettingsController::new(
            dir.path().to_path_buf(),
            agent_policy.clone(),
        ),
    );
    let agent_settings_for_server = agent_settings.clone();
    let capture_provider: Arc<dyn ScreenCaptureProvider> = Arc::new(FakeCaptureProvider);
    let capture_provider_for_server = capture_provider.clone();
    let service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        move || {
            Ok(local_workspace_mcp_manager::server::WorkspaceServer::new(
                core_for_server.clone(),
                agent_policy_for_server.clone(),
                agent_settings_for_server.clone(),
                capture_provider_for_server.clone(),
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
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
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
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
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
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
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
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
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
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
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
    assert_audit_row(
        &core,
        "read_file",
        &ws_id,
        "error",
        Some("PATH_OUTSIDE_WORKSPACE"),
    );
}

/// Scenario C: sensitive files (.env) return SENSITIVE_FILE_BLOCKED.
#[tokio::test]
async fn sensitive_file_blocked_over_mcp() {
    let (port, core, dir, ws_id) = spawn_server().await;
    std::fs::write(dir.path().join("ws/.env"), "SECRET=1\n").unwrap();
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
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
    assert_audit_row(
        &core,
        "read_file",
        &ws_id,
        "error",
        Some("SENSITIVE_FILE_BLOCKED"),
    );
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
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
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
    let body_text = res
        .content
        .first()
        .and_then(|c| c.as_text().map(|t| t.text.to_string()))
        .unwrap_or_default();
    let rev: String = extract_json(&body_text)["revision"]
        .as_str()
        .unwrap()
        .into();

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
    assert_audit_row(
        &core,
        "apply_patch",
        &ws_id,
        "error",
        Some("FILE_CHANGED_SINCE_READ"),
    );
}

// ---- Task 13C: execution tools (run_command / run_powershell) ----

/// Grants command execution on a workspace with the given mode.
fn enable_execute(core: &Core, ws_id: &str, mode: lc_core::models::ExecutionMode) {
    lc_core::workspace_repo::update_execution_access(&core.db, ws_id, true, mode).unwrap();
}

fn detail_keys(detail: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = detail
        .as_object()
        .expect("audit detail is an object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

#[tokio::test]
async fn run_command_without_execute_is_denied() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("run_command").with_arguments(
                serde_json::json!({ "workspace_id": ws_id, "program": "git", "args": ["--version"] })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false));
    assert!(text.contains("EXECUTE_DISABLED"), "{text}");
    assert_audit_row(
        &core,
        "run_command",
        &ws_id,
        "deny",
        Some("EXECUTE_DISABLED"),
    );
}

#[tokio::test]
async fn run_command_approved_runs_and_audits_safe_metadata() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    enable_execute(
        &core,
        &ws_id,
        lc_core::models::ExecutionMode::ApprovedCommands,
    );
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("run_command").with_arguments(
                serde_json::json!({ "workspace_id": ws_id, "program": "git", "args": ["--version"] })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let body_text = res
        .content
        .first()
        .and_then(|c| c.as_text().map(|t| t.text.to_string()))
        .unwrap_or_default();
    assert!(!res.is_error.unwrap_or(false), "run failed: {body_text}");
    let body = extract_json(&body_text);
    assert_eq!(body["exit_code"], 0);
    assert!(body["stdout"].as_str().unwrap().contains("git version"));
    assert_eq!(body["timed_out"], false);
    assert_eq!(body["stderr_truncated"], false);
    assert!(body["duration_ms"].as_u64().is_some());

    // Audit: allow row carrying ONLY the safe metadata keys, never output.
    assert_audit_row(&core, "run_command", &ws_id, "allow", None);
    let row = latest_audit_row(&core, "run_command");
    assert!(row.path.is_none(), "execution rows carry no path");
    let detail = row.detail.as_ref().expect("run_command detail");
    assert_eq!(
        detail_keys(detail),
        vec![
            "arg_count",
            "cwd",
            "duration_ms",
            "exit_code",
            "program",
            "stderr_truncated",
            "stdout_truncated",
            "timed_out"
        ]
    );
    assert_eq!(detail["program"], "git");
    assert_eq!(detail["arg_count"], 1);
    assert_eq!(detail["cwd"], ".");
    assert_eq!(detail["timed_out"], false);
    // stdout/stderr content must never reach the audit log.
    let detail_str = detail.to_string();
    assert!(
        !detail_str.contains("git version"),
        "audit leaked stdout: {detail_str}"
    );
}

#[tokio::test]
async fn run_command_unapproved_program_is_command_not_allowed() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    enable_execute(
        &core,
        &ws_id,
        lc_core::models::ExecutionMode::ApprovedCommands,
    );
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("run_command").with_arguments(
                serde_json::json!({ "workspace_id": ws_id, "program": "cmd.exe" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false));
    assert!(text.contains("COMMAND_NOT_ALLOWED"), "{text}");
    assert_audit_row(
        &core,
        "run_command",
        &ws_id,
        "error",
        Some("COMMAND_NOT_ALLOWED"),
    );
}

#[cfg(windows)]
#[tokio::test]
async fn run_powershell_requires_full_powershell_mode() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    enable_execute(
        &core,
        &ws_id,
        lc_core::models::ExecutionMode::ApprovedCommands,
    );
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("run_powershell").with_arguments(
                serde_json::json!({ "workspace_id": ws_id, "script": "Write-Output ok" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false));
    assert!(text.contains("POWERSHELL_DISABLED"), "{text}");
    // Mode check lives inside the audited closure, so it lands as an
    // "error"-classified row (pipeline convention for post-prelude failures),
    // unlike the prelude capability denies (EXECUTE_DISABLED).
    assert_audit_row(
        &core,
        "run_powershell",
        &ws_id,
        "error",
        Some("POWERSHELL_DISABLED"),
    );
}

#[cfg(windows)]
#[tokio::test]
async fn run_powershell_full_mode_runs_and_audits_safe_metadata() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    enable_execute(
        &core,
        &ws_id,
        lc_core::models::ExecutionMode::FullPowerShell,
    );
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let script = "Write-Output ('ps-ok-' + (2 + 3))";
    let res = client
        .call_tool(
            CallToolRequestParams::new("run_powershell").with_arguments(
                serde_json::json!({
                    "workspace_id": ws_id,
                    "script": script
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    let body_text = res
        .content
        .first()
        .and_then(|c| c.as_text().map(|t| t.text.to_string()))
        .unwrap_or_default();
    assert!(!res.is_error.unwrap_or(false), "ps failed: {body_text}");
    let body = extract_json(&body_text);
    assert_eq!(body["exit_code"], 0);
    assert!(body["stdout"].as_str().unwrap().contains("ps-ok-5"));

    // Audit: hash + size of the script, never the script itself or output.
    assert_audit_row(&core, "run_powershell", &ws_id, "allow", None);
    let row = latest_audit_row(&core, "run_powershell");
    assert!(row.path.is_none());
    let detail = row.detail.as_ref().expect("run_powershell detail");
    assert_eq!(
        detail_keys(detail),
        vec![
            "cwd",
            "duration_ms",
            "exit_code",
            "script_bytes",
            "script_sha256",
            "stderr_truncated",
            "stdout_truncated",
            "timed_out"
        ]
    );
    assert_eq!(detail["script_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(detail["script_bytes"], script.len() as u64);
    let detail_str = detail.to_string();
    assert!(
        !detail_str.contains("ps-ok") && !detail_str.contains("Write-Output"),
        "audit leaked script/output: {detail_str}"
    );
}

#[tokio::test]
async fn pause_blocks_execution_tools() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    enable_execute(
        &core,
        &ws_id,
        lc_core::models::ExecutionMode::ApprovedCommands,
    );
    core.pause.set(true);
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    for tool in ["run_command", "run_powershell"] {
        let args = if tool == "run_command" {
            serde_json::json!({ "workspace_id": ws_id, "program": "git" })
        } else {
            serde_json::json!({ "workspace_id": ws_id, "script": "Get-Date" })
        };
        let res = client
            .call_tool(
                CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&res).unwrap();
        assert!(res.is_error.unwrap_or(false));
        assert!(text.contains("ACCESS_PAUSED"), "{tool}: {text}");
        // Pause rows are deny rows with NULL workspace_id (kill switch wins).
        let row = latest_audit_row(&core, tool);
        assert_eq!(row.decision, "deny");
        assert_eq!(row.error_code.as_deref(), Some("ACCESS_PAUSED"));
        assert_eq!(row.workspace_id, None);
    }
}

#[tokio::test]
async fn run_command_cwd_escape_is_path_outside_workspace() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    enable_execute(
        &core,
        &ws_id,
        lc_core::models::ExecutionMode::ApprovedCommands,
    );
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("run_command").with_arguments(
                serde_json::json!({
                    "workspace_id": ws_id, "program": "git", "cwd": ".."
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
    assert_audit_row(
        &core,
        "run_command",
        &ws_id,
        "error",
        Some("PATH_OUTSIDE_WORKSPACE"),
    );
}

#[tokio::test]
async fn run_command_timeout_over_limit_is_invalid_argument() {
    let (port, core, _dir, ws_id) = spawn_server().await;
    enable_execute(
        &core,
        &ws_id,
        lc_core::models::ExecutionMode::ApprovedCommands,
    );
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("run_command").with_arguments(
                serde_json::json!({
                    "workspace_id": ws_id, "program": "git",
                    "args": ["--version"], "timeout_ms": 999_999_999u64
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
    assert!(text.contains("INVALID_ARGUMENT"), "{text}");
    assert_audit_row(
        &core,
        "run_command",
        &ws_id,
        "error",
        Some("INVALID_ARGUMENT"),
    );
}

/// Tool discovery exposes the file/execution surface plus live agent policy.
#[tokio::test]
async fn tool_discovery_exposes_agent_policy_tool() {
    let (port, _core, _dir, _ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();
    let mut names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "apply_patch",
            "capture_screen",
            "create_file",
            "find_files",
            "get_agent_policy",
            "list_capture_targets",
            "list_directory",
            "list_workspaces",
            "read_file",
            "read_files",
            "run_command",
            "run_powershell",
            "search_text",
            "update_agent_settings",
        ]
    );
}

#[tokio::test]
async fn get_agent_policy_returns_secure_live_defaults() {
    let (port, _core, _dir, _ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(CallToolRequestParams::new("get_agent_policy"))
        .await
        .unwrap();
    assert!(!res.is_error.unwrap_or(false));
    let text = serde_json::to_string(&res).unwrap();
    assert!(text.contains("\"mode\":\"autonomous\""), "{text}");
    assert!(text.contains("\"paused\":false"), "{text}");
    assert!(text.contains("\"unrestricted_access\":false"), "{text}");
    assert!(text.contains("\"screen_capture_enabled\":false"), "{text}");
    assert!(
        text.contains("\"capture_scope\":\"windows_and_displays\""),
        "{text}"
    );
    assert!(text.contains("\"visual_verification\":true"), "{text}");
}

#[tokio::test]
async fn update_agent_settings_changes_guidance_and_persists() {
    let (port, core, dir, _ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("update_agent_settings").with_arguments(
                serde_json::json!({
                    "agent_mode": "conservative",
                    "visual_verification": false
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(!res.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("\"mode\":\"conservative\""), "{text}");
    assert!(text.contains("\"visual_verification\":false"), "{text}");

    let saved = local_workspace_mcp_manager::settings::load_app_settings(dir.path()).unwrap();
    assert_eq!(
        saved.agent_mode,
        local_workspace_mcp_manager::settings::AgentMode::Conservative
    );
    assert!(!saved.visual_verification);
    assert!(!saved.screen_capture_enabled);
    assert_eq!(
        saved.capture_scope,
        local_workspace_mcp_manager::settings::CaptureScope::WindowsAndDisplays
    );

    let live = client
        .call_tool(CallToolRequestParams::new("get_agent_policy"))
        .await
        .unwrap();
    let live_text = serde_json::to_string(&live).unwrap();
    assert!(
        live_text.contains("\"mode\":\"conservative\""),
        "{live_text}"
    );
    assert!(
        live_text.contains("\"visual_verification\":false"),
        "{live_text}"
    );

    let row = latest_audit_row(&core, "update_agent_settings");
    assert_eq!(row.decision, "allow");
    assert_eq!(row.workspace_id, None);
    let detail = row.detail.as_ref().expect("settings audit detail");
    assert!(detail.to_string().contains("agent_mode"));
    assert!(detail.to_string().contains("visual_verification"));
}

#[tokio::test]
async fn update_agent_settings_can_reduce_capture_privilege() {
    let settings = local_workspace_mcp_manager::settings::AppSettings {
        screen_capture_enabled: true,
        capture_scope: local_workspace_mcp_manager::settings::CaptureScope::WindowsAndDisplays,
        ..Default::default()
    };
    let (port, _core, dir, _ws_id) = spawn_server_with_settings(settings).await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("update_agent_settings").with_arguments(
                serde_json::json!({
                    "screen_capture_enabled": false,
                    "capture_scope": "windows_only"
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
        "{}",
        serde_json::to_string(&res).unwrap()
    );
    let saved = local_workspace_mcp_manager::settings::load_app_settings(dir.path()).unwrap();
    assert!(!saved.screen_capture_enabled);
    assert_eq!(
        saved.capture_scope,
        local_workspace_mcp_manager::settings::CaptureScope::WindowsOnly
    );
}

#[tokio::test]
async fn update_agent_settings_cannot_enable_capture() {
    let (port, core, dir, _ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();
    let res = client
        .call_tool(
            CallToolRequestParams::new("update_agent_settings").with_arguments(
                serde_json::json!({ "screen_capture_enabled": true })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("SETTINGS_ELEVATION_REQUIRES_USER"), "{text}");
    assert!(
        !local_workspace_mcp_manager::settings::load_app_settings(dir.path())
            .unwrap()
            .screen_capture_enabled
    );
    let row = latest_audit_row(&core, "update_agent_settings");
    assert_eq!(row.decision, "deny");
    assert_eq!(
        row.error_code.as_deref(),
        Some("SETTINGS_ELEVATION_REQUIRES_USER")
    );
}

#[tokio::test]
async fn update_agent_settings_cannot_broaden_capture_scope() {
    let settings = local_workspace_mcp_manager::settings::AppSettings {
        screen_capture_enabled: true,
        capture_scope: local_workspace_mcp_manager::settings::CaptureScope::WindowsOnly,
        ..Default::default()
    };
    let (port, core, dir, _ws_id) = spawn_server_with_settings(settings).await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();
    let res = client
        .call_tool(
            CallToolRequestParams::new("update_agent_settings").with_arguments(
                serde_json::json!({ "capture_scope": "windows_and_displays" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("SETTINGS_ELEVATION_REQUIRES_USER"), "{text}");
    let saved = local_workspace_mcp_manager::settings::load_app_settings(dir.path()).unwrap();
    assert_eq!(
        saved.capture_scope,
        local_workspace_mcp_manager::settings::CaptureScope::WindowsOnly
    );
    let row = latest_audit_row(&core, "update_agent_settings");
    assert_eq!(row.decision, "deny");
}

#[tokio::test]
async fn update_agent_settings_pause_wins() {
    let (port, core, dir, _ws_id) = spawn_server().await;
    core.pause.set(true);
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();
    let res = client
        .call_tool(
            CallToolRequestParams::new("update_agent_settings").with_arguments(
                serde_json::json!({ "agent_mode": "conservative" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("ACCESS_PAUSED"), "{text}");
    assert_eq!(
        local_workspace_mcp_manager::settings::load_app_settings(dir.path())
            .unwrap()
            .agent_mode,
        local_workspace_mcp_manager::settings::AgentMode::Autonomous
    );
    let row = latest_audit_row(&core, "update_agent_settings");
    assert_eq!(row.decision, "deny");
    assert_eq!(row.error_code.as_deref(), Some("ACCESS_PAUSED"));
}

#[tokio::test]
async fn update_agent_settings_rejects_empty_patch() {
    let (port, core, dir, _ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();
    let res = client
        .call_tool(
            CallToolRequestParams::new("update_agent_settings")
                .with_arguments(serde_json::json!({}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("INVALID_ARGUMENT"), "{text}");
    assert_eq!(
        local_workspace_mcp_manager::settings::load_app_settings(dir.path()).unwrap(),
        local_workspace_mcp_manager::settings::AppSettings::default()
    );
    let row = latest_audit_row(&core, "update_agent_settings");
    assert_eq!(row.decision, "error");
    assert_eq!(row.error_code.as_deref(), Some("INVALID_ARGUMENT"));
}

#[tokio::test]
async fn capture_tools_are_denied_when_capture_is_disabled() {
    let (port, core, _dir, _ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    for (tool, args) in [
        ("list_capture_targets", serde_json::json!({})),
        (
            "capture_screen",
            serde_json::json!({ "target_id": "window:7" }),
        ),
    ] {
        let res = client
            .call_tool(
                CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&res).unwrap();
        assert!(res.is_error.unwrap_or(false), "{tool}: {text}");
        assert!(text.contains("SCREEN_CAPTURE_DISABLED"), "{tool}: {text}");
        let row = latest_audit_row(&core, tool);
        assert_eq!(row.decision, "deny");
        assert_eq!(row.error_code.as_deref(), Some("SCREEN_CAPTURE_DISABLED"));
        assert_eq!(row.workspace_id, None);
    }
}

#[tokio::test]
async fn capture_pause_wins_over_enabled_permission() {
    let settings = local_workspace_mcp_manager::settings::AppSettings {
        screen_capture_enabled: true,
        ..Default::default()
    };
    let (port, core, _dir, _ws_id) = spawn_server_with_settings(settings).await;
    core.pause.set(true);
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("capture_screen").with_arguments(
                serde_json::json!({ "target_id": "window:7" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&res).unwrap();
    assert!(res.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("ACCESS_PAUSED"), "{text}");
    let row = latest_audit_row(&core, "capture_screen");
    assert_eq!(row.decision, "deny");
    assert_eq!(row.error_code.as_deref(), Some("ACCESS_PAUSED"));
}

#[tokio::test]
async fn list_capture_targets_honors_windows_only_scope() {
    let settings = local_workspace_mcp_manager::settings::AppSettings {
        screen_capture_enabled: true,
        capture_scope: local_workspace_mcp_manager::settings::CaptureScope::WindowsOnly,
        ..Default::default()
    };
    let (port, core, _dir, _ws_id) = spawn_server_with_settings(settings).await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(CallToolRequestParams::new("list_capture_targets"))
        .await
        .unwrap();
    assert!(!res.is_error.unwrap_or(false));
    let body_text = res
        .content
        .first()
        .and_then(|c| c.as_text().map(|t| t.text.to_string()))
        .unwrap_or_default();
    let body = extract_json(&body_text);
    let targets = body.as_array().expect("capture target array");
    assert_eq!(targets.len(), 1, "{body_text}");
    assert_eq!(targets[0]["id"], "window:7");
    assert_eq!(targets[0]["kind"], "window");

    let row = latest_audit_row(&core, "list_capture_targets");
    assert_eq!(row.decision, "allow");
    assert_eq!(row.detail.as_ref().unwrap()["count"], 1);
}

#[tokio::test]
async fn capture_screen_returns_png_image_metadata_and_safe_audit() {
    let settings = local_workspace_mcp_manager::settings::AppSettings {
        screen_capture_enabled: true,
        ..Default::default()
    };
    let (port, core, _dir, _ws_id) = spawn_server_with_settings(settings).await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("capture_screen").with_arguments(
                serde_json::json!({ "target_id": "window:7", "max_edge": 100 })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let serialized = serde_json::to_string(&res).unwrap();
    assert!(!res.is_error.unwrap_or(false), "{serialized}");
    let png = fake_png();
    let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
    let sha256 = lc_core::ops::sha256_hex(&png);
    assert!(serialized.contains("image/png"), "{serialized}");
    assert!(
        serialized.contains(&encoded),
        "image bytes missing: {serialized}"
    );
    assert!(serialized.contains(&sha256), "sha missing: {serialized}");
    assert!(serialized.contains("\"original_width\":2"), "{serialized}");
    assert!(serialized.contains("\"returned_height\":1"), "{serialized}");

    let row = latest_audit_row(&core, "capture_screen");
    assert_eq!(row.decision, "allow");
    assert_eq!(row.workspace_id, None);
    let detail = row.detail.as_ref().expect("capture audit detail");
    assert_eq!(detail["target_id"], "window:7");
    assert_eq!(detail["target_kind"], "window");
    assert_eq!(detail["original_width"], 2);
    assert_eq!(detail["original_height"], 1);
    assert_eq!(detail["returned_width"], 2);
    assert_eq!(detail["returned_height"], 1);
    assert_eq!(detail["encoded_bytes"], png.len() as u64);
    assert_eq!(detail["sha256"], sha256);
    let detail_text = detail.to_string();
    assert!(!detail_text.contains(&encoded), "audit leaked image bytes");
    assert!(!detail_text.contains("Editor") && !detail_text.contains("TestApp"));
}

#[tokio::test]
async fn server_instructions_define_the_agent_operating_loop() {
    let (port, _core, _dir, _ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let peer = client.peer_info().expect("server peer info");
    let instructions = peer.instructions.as_deref().unwrap_or_default();
    for required in [
        "DISCOVER",
        "PLAN",
        "INSPECT",
        "ACT",
        "VERIFY",
        "RECOVER",
        "REPORT",
        "FILE_CHANGED_SINCE_READ",
        "get_agent_policy",
        "update_agent_settings",
        "desktop Settings",
        "cannot enable screen capture",
        "capture_screen",
        "visual evidence",
        "disabled screen capture",
        "Never bypass",
    ] {
        assert!(
            instructions.contains(required),
            "agent instructions missing {required:?}: {instructions}"
        );
    }
}

#[tokio::test]
async fn prompt_discovery_exposes_the_agent_skill_pack() {
    let (port, _core, _dir, _ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let mut names: Vec<String> = client
        .list_all_prompts()
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "agent-task",
            "debug-fix",
            "explore-project",
            "implement-change",
            "review-verify",
        ]
    );
}

#[tokio::test]
async fn agent_task_prompt_embeds_goal_and_autonomous_completion_contract() {
    let (port, _core, _dir, _ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    let result = client
        .get_prompt(
            GetPromptRequestParams::new("agent-task").with_arguments(
                serde_json::json!({ "task": "fix the flaky parser test" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let text = result
        .messages
        .first()
        .and_then(|m| m.content.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or_default();
    assert!(text.contains("fix the flaky parser test"), "{text}");
    assert!(text.contains("work autonomously"), "{text}");
    assert!(text.contains("verify"), "{text}");
    assert!(
        text.contains("blocked by user-controlled permissions"),
        "{text}"
    );
}

#[tokio::test]
async fn tool_descriptions_teach_the_model_safe_composition() {
    let (port, _core, _dir, _ws_id) = spawn_server().await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();
    let tools = client.list_all_tools().await.unwrap();
    let description = |name: &str| {
        tools
            .iter()
            .find(|t| t.name.as_ref() == name)
            .and_then(|t| t.description.as_deref())
            .unwrap_or_default()
    };

    assert!(description("search_text").contains("Prefer this before reading many files"));
    assert!(description("read_file").contains("before apply_patch"));
    assert!(description("apply_patch").contains("FILE_CHANGED_SINCE_READ"));
    assert!(description("run_command").contains("verify changes"));
    assert!(description("run_powershell").contains("Only use when"));
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
        .serve(StreamableHttpClientTransport::from_uri(local_test_mcp_uri(
            port,
        )))
        .await
        .unwrap();

    // Absolute path into workspace A, presented under workspace B's id.
    let abs = dir
        .path()
        .join("ws/hello.txt")
        .display()
        .to_string()
        .replace('\\', "/");
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
