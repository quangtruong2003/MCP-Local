use std::{
    collections::HashMap,
    io::Cursor,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use base64::Engine as _;
use image::{DynamicImage, ImageFormat};

use lc_core::error::{CoreError, ErrorCode};
use local_workspace_mcp_manager::{
    browser::{
        driver::{BrowserPageDriver, BrowserPageDriverFactory, PageDriverStatus, PageLaunchSpec},
        manager::{BrowserManager, CookieStore, RuntimeGate},
    },
    screen_capture::{CaptureTarget, CapturedImage, ScreenCaptureProvider},
    settings::{AgentPolicyState, AgentSettingsController, AppSettings},
};
use rmcp::{model::CallToolRequestParams, transport::StreamableHttpClientTransport, ServiceExt};

#[derive(Debug, Clone)]
struct RecordedCall {
    tool: String,
    arguments: serde_json::Value,
}

/// Shared fake state for all page children of one test session. One logical
/// page = one FakePageDriver, but recording/injection is shared so tests can
/// observe every child through one handle.
#[derive(Debug, Default)]
struct FakeBrowser {
    calls: std::sync::Mutex<Vec<RecordedCall>>,
    failures: std::sync::Mutex<Vec<(String, lc_core::error::CoreError)>>,
    responses: std::sync::Mutex<Vec<(String, String)>>,
    screenshot_block: std::sync::Mutex<Option<rmcp::model::ContentBlock>>,
    starts: AtomicUsize,
    stops: AtomicUsize,
}

impl FakeBrowser {
    fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }

    /// Injects a one-shot driver error for the next call of `tool`.
    fn fail(&self, tool: &str, error: lc_core::error::CoreError) {
        self.failures
            .lock()
            .unwrap()
            .push((tool.to_string(), error));
    }

    /// Injects a one-shot canned text response for the next call of `tool`.
    fn respond(&self, tool: &str, text: String) {
        self.responses
            .lock()
            .unwrap()
            .push((tool.to_string(), text));
    }

    fn png_block() -> rmcp::model::ContentBlock {
        let image = DynamicImage::new_rgba8(4, 3);
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        rmcp::model::ContentBlock::image(
            base64::engine::general_purpose::STANDARD.encode(bytes),
            "image/png",
        )
    }
}

struct FakePageDriver {
    shared: Arc<FakeBrowser>,
}

#[async_trait::async_trait]
impl BrowserPageDriver for FakePageDriver {
    async fn start(&self, spec: PageLaunchSpec) -> Result<PageDriverStatus, CoreError> {
        self.shared.starts.fetch_add(1, Ordering::SeqCst);
        Ok(PageDriverStatus {
            running: true,
            page_id: spec.page_id,
            child_pid: None,
        })
    }

    async fn call(
        &self,
        tool: &'static str,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> Result<rmcp::model::CallToolResult, CoreError> {
        self.shared.calls.lock().unwrap().push(RecordedCall {
            tool: tool.to_string(),
            arguments: serde_json::Value::Object(arguments.clone()),
        });

        let failure_pos = {
            self.shared
                .failures
                .lock()
                .unwrap()
                .iter()
                .position(|(name, _)| name == tool)
        };
        if let Some(position) = failure_pos {
            let (_, error) = self.shared.failures.lock().unwrap().remove(position);
            return Err(error);
        }
        let response_pos = {
            self.shared
                .responses
                .lock()
                .unwrap()
                .iter()
                .position(|(name, _)| name == tool)
        };
        if let Some(position) = response_pos {
            let (_, text) = self.shared.responses.lock().unwrap().remove(position);
            return Ok(rmcp::model::CallToolResult::success(vec![
                rmcp::model::ContentBlock::text(text),
            ]));
        }

        match tool {
            // Mirrors Lightpanda 0.3.7 `tree` output: one space of indent per
            // depth level, then a decimal backendNodeId followed by
            // interactivity/role/name/value markers.
            "tree" => return Ok(rmcp::model::CallToolResult::success(vec![rmcp::model::ContentBlock::text("1 WebArea 'Browser Test App'\n 2 [i] textbox 'Name'\n 3 [i] combobox 'Color' options=['red','green','blue']\n 4 [i] button 'Submit'\n")])),
            // Mirrors Lightpanda 0.3.7 `nodeDetails`: JSON carrying a
            // ready-to-use CSS selector for the backend node.
            "nodeDetails" => {
                let id = arguments
                    .get("backendNodeId")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                return Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::ContentBlock::text(format!(
                        "{{\"backendNodeId\":{id},\"tagName\":\"div\",\"role\":\"generic\",\"isInteractive\":true,\"selector\":\"#nd-{id}\"}}"
                    )),
                ]));
            }
            // Mirrors a real page: the current URL is the last goto target.
            "getUrl" => {
                let calls = self.shared.calls.lock().unwrap();
                let last_url = calls
                    .iter()
                    .rev()
                    .filter(|c| c.tool == "goto")
                    .find_map(|c| c.arguments.get("url").and_then(|v| v.as_str()))
                    .unwrap_or("about:blank");
                return Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::ContentBlock::text(last_url.to_string()),
                ]));
            }
            "browser_take_screenshot" => {
                let block = self.shared.screenshot_block.lock().unwrap().take();
                return Ok(rmcp::model::CallToolResult::success(vec![
                    block.unwrap_or_else(FakeBrowser::png_block),
                ]));
            }
            // Mirrors Lightpanda 0.3.7 `consoleLogs`: `[tag] text` lines, one
            // batch per call, drained from the child's buffer. A fresh session
            // reports no messages unless a test queues a batch via respond().
            "consoleLogs" => return Ok(rmcp::model::CallToolResult::success(vec![rmcp::model::ContentBlock::text("No console messages.")])),
            "evaluate" => {
                let script = arguments.get("script").and_then(|v| v.as_str()).unwrap_or_default();
                let text = if script.contains("slice(-200)") { "{\"resources\":[]}" }
                    else if script.contains("performance.getEntriesByType") { "{\"navigation\":[],\"resources\":[]}" }
                    else if script.contains("clone.outerHTML") { "{\"outerHTML\":\"<input type=\\\"password\\\" value=\\\"[redacted]\\\">\",\"style\":null}" }
                    else if script.contains("DragEvent") || script.contains("MouseEvent") { "true" }
                    else { "eval-ok" };
                return Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::ContentBlock::text(text),
                ]));
            }
            "getCookies" => {
                return Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::ContentBlock::text("{\"cookies\":[]}"),
                ]));
            }
            _ => {}
        }

        Ok(rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::text("ok"),
        ]))
    }

    async fn stop(&self) -> Result<(), CoreError> {
        self.shared.stops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn is_alive(&self) -> bool {
        true
    }
}

struct FakePageFactory {
    shared: Arc<FakeBrowser>,
}

impl BrowserPageDriverFactory for FakePageFactory {
    fn create(&self) -> Arc<dyn BrowserPageDriver> {
        Arc::new(FakePageDriver {
            shared: self.shared.clone(),
        })
    }
}

struct OkRuntimeGate;

#[async_trait::async_trait]
impl RuntimeGate for OkRuntimeGate {
    async fn ensure_ready(&self) -> Result<(), CoreError> {
        Ok(())
    }
}

#[derive(Default)]
struct MemoryCookieStore {
    files: std::sync::Mutex<HashMap<String, String>>,
}

#[async_trait::async_trait]
impl CookieStore for MemoryCookieStore {
    async fn write(&self, path: &str, json: &str) -> Result<(), CoreError> {
        self.files
            .lock()
            .unwrap()
            .insert(path.to_string(), json.to_string());
        Ok(())
    }

    async fn read(&self, path: &str) -> Result<Option<String>, CoreError> {
        Ok(self.files.lock().unwrap().get(path).cloned())
    }
}

#[derive(Default)]
struct EmptyCapture;
impl ScreenCaptureProvider for EmptyCapture {
    fn list_targets(&self) -> Result<Vec<CaptureTarget>, lc_core::error::CoreError> {
        Ok(vec![])
    }
    fn capture(
        &self,
        _target_id: &str,
        _max_edge: Option<u32>,
    ) -> Result<CapturedImage, lc_core::error::CoreError> {
        unreachable!()
    }
}

async fn spawn(
    settings: AppSettings,
) -> (
    u16,
    tempfile::TempDir,
    Arc<lc_core::CoreServices>,
    Arc<FakeBrowser>,
    Arc<BrowserManager>,
) {
    let dir = tempfile::tempdir().unwrap();
    local_workspace_mcp_manager::settings::save_app_settings(dir.path(), &settings).unwrap();
    let core = Arc::new(lc_core::CoreServices::open(&dir.path().join("test.db")).unwrap());
    let policy = Arc::new(AgentPolicyState::from_settings(&settings));
    let controller = Arc::new(AgentSettingsController::new(
        dir.path().to_path_buf(),
        policy.clone(),
    ));
    let fake = Arc::new(FakeBrowser::default());
    let browser = Arc::new(
        BrowserManager::new(
            core.clone(),
            dir.path().to_path_buf(),
            &settings,
            Arc::new(FakePageFactory {
                shared: fake.clone(),
            }),
            Arc::new(OkRuntimeGate),
            Arc::new(MemoryCookieStore::default()),
        )
        .unwrap(),
    );
    let capture: Arc<dyn ScreenCaptureProvider> = Arc::new(EmptyCapture);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let core_for_server = core.clone();
    let browser_for_server = browser.clone();
    let service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        move || {
            Ok(local_workspace_mcp_manager::server::WorkspaceServer::new(
                core_for_server.clone(),
                policy.clone(),
                controller.clone(),
                browser_for_server.clone(),
                capture.clone(),
            ))
        },
        Arc::new(
            rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(),
        ),
        Default::default(),
    );
    let app = axum::Router::new().nest_service("/mcp", service);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (port, dir, core, fake, browser)
}

async fn client(port: u16) -> rmcp::service::RunningService<rmcp::service::RoleClient, ()> {
    ().serve(StreamableHttpClientTransport::from_uri(format!(
        "http://127.0.0.1:{port}/mcp"
    )))
    .await
    .unwrap()
}

fn arguments(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().unwrap().clone()
}

/// The newest session artifact dir under the temp app-data root. Sessions are
/// created one per test, so the newest dir is the active session's.
fn active_artifact_dir(dir: &tempfile::TempDir) -> PathBuf {
    let root = dir.path().join("browser").join("artifacts");
    std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .max()
        .expect("session artifact dir must exist")
}

fn matching_audit(core: &lc_core::CoreServices, op: &str) -> Vec<lc_core::models::AuditEvent> {
    lc_core::audit::list_recent(&core.db, 100)
        .unwrap()
        .into_iter()
        .filter(|row| row.op == op)
        .collect()
}

/// The `text` field of a BrowserTextTruncatedOutput structured result, free of
/// the outer JSON escaping that `serde_json::to_string(&result)` would add.
fn truncated_text(result: &rmcp::model::CallToolResult) -> String {
    result.structured_content.as_ref().unwrap()["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn browser_discovery_exposes_exactly_39_public_browser_tools() {
    let (port, _dir, _core, _driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let tools = client.list_all_tools().await.unwrap();
    let missing_output_schema: Vec<String> = tools
        .iter()
        .filter(|tool| tool.name.starts_with("browser_"))
        .filter(|tool| tool.output_schema.is_none())
        .map(|tool| tool.name.to_string())
        .collect();
    assert!(
        missing_output_schema.is_empty(),
        "every browser tool must declare an outputSchema: {missing_output_schema:?}"
    );
    // Upstream Lightpanda admin/state tools are internal-only: they leak
    // environment, cookies, and session control and must never be discovered.
    let forbidden = [
        "getEnv",
        "getCookies",
        "save",
        "session_new",
        "session_list",
        "session_close",
    ];
    let exposed_admin: Vec<String> = tools
        .iter()
        .map(|tool| tool.name.to_string())
        .filter(|name| forbidden.contains(&name.as_str()))
        .collect();
    assert!(
        exposed_admin.is_empty(),
        "upstream admin tools must never be exposed: {exposed_admin:?}"
    );
    let names: std::collections::BTreeSet<String> = tools
        .into_iter()
        .map(|tool| tool.name.to_string())
        .filter(|name| name.starts_with("browser_"))
        .collect();
    let expected: std::collections::BTreeSet<String> = [
        "browser_get_status",
        "browser_open",
        "browser_close",
        "browser_tabs",
        "browser_navigate",
        "browser_search",
        "browser_snapshot",
        "browser_click",
        "browser_fill",
        "browser_press",
        "browser_select",
        "browser_hover",
        "browser_drag",
        "browser_wait",
        "browser_dialog",
        "browser_set_viewport",
        "browser_emulate_device",
        "browser_screenshot",
        "browser_console",
        "browser_network",
        "browser_dom",
        "browser_evaluate",
        "browser_performance",
        "browser_trace",
        "browser_devtools",
        "browser_upload",
        "browser_downloads",
        "browser_save_download_to_workspace",
        "browser_run_steps",
        "browser_markdown",
        "browser_html",
        "browser_links",
        "browser_find_element",
        "browser_interactive_elements",
        "browser_extract",
        "browser_structured_data",
        "browser_detect_forms",
        "browser_scroll",
        "browser_set_checked",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(names.len(), 39, "browser tool count changed: {names:?}");
    assert_eq!(names, expected);
}

#[tokio::test]
async fn task7_to_11_browser_tools_smoke_through_mcp_and_preserve_redaction() {
    let (port, dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let workspace_root = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace_root).unwrap();
    std::fs::write(workspace_root.join("upload.txt"), b"upload-body").unwrap();
    let ws = lc_core::workspace_repo::insert_workspace(
        &core.db,
        "Web",
        &workspace_root.to_string_lossy(),
    )
    .unwrap();
    lc_core::workspace_repo::update_capabilities(&core.db, &ws.id, true, true).unwrap();
    let client = client(port).await;

    let open = client
        .call_tool(CallToolRequestParams::new("browser_open"))
        .await
        .unwrap();
    assert!(!open.is_error.unwrap_or(false));

    let cases = [
        ("browser_snapshot", serde_json::json!({})),
        ("browser_click", serde_json::json!({"ref":"e1"})),
        (
            "browser_fill",
            serde_json::json!({"ref":"e2","text":"super-secret-value"}),
        ),
        ("browser_press", serde_json::json!({"key":"Enter"})),
        (
            "browser_select",
            serde_json::json!({"ref":"e3","values":["US"]}),
        ),
        ("browser_hover", serde_json::json!({"ref":"e1"})),
        (
            "browser_drag",
            serde_json::json!({"start_ref":"e1","end_ref":"e3"}),
        ),
        (
            "browser_wait",
            serde_json::json!({"condition":{"kind":"delay","milliseconds":1},"timeout_ms":1000}),
        ),
        ("browser_console", serde_json::json!({"limit":100})),
        ("browser_network", serde_json::json!({"limit":100})),
        ("browser_dom", serde_json::json!({"ref":"e2"})),
        (
            "browser_evaluate",
            serde_json::json!({"expression":"() => document.title"}),
        ),
        ("browser_performance", serde_json::json!({})),
        ("browser_trace", serde_json::json!({"action":"start"})),
    ];
    for (name, args) in cases {
        let result = client
            .call_tool(CallToolRequestParams::new(name).with_arguments(arguments(args)))
            .await
            .unwrap();
        assert!(
            !result.is_error.unwrap_or(false),
            "{name}: {}",
            serde_json::to_string(&result).unwrap()
        );
        // Side-effecting interactions invalidate snapshot refs; the next ref
        // use must re-snapshot first, exactly like a real agent flow.
        if matches!(
            name,
            "browser_click" | "browser_fill" | "browser_select" | "browser_press" | "browser_drag"
        ) {
            let resnap = client
                .call_tool(CallToolRequestParams::new("browser_snapshot"))
                .await
                .unwrap();
            assert!(!resnap.is_error.unwrap_or(false));
        }
    }

    // Viewport resizing is structurally unsupported under the lightpanda
    // runtime and must fail closed without any upstream side effect.
    let calls_before = driver.calls().len();
    let viewport = client
        .call_tool(
            CallToolRequestParams::new("browser_set_viewport")
                .with_arguments(arguments(serde_json::json!({"width":375,"height":812}))),
        )
        .await
        .unwrap();
    let viewport_text = serde_json::to_string(&viewport).unwrap();
    assert!(viewport.is_error.unwrap_or(false), "{viewport_text}");
    assert!(
        viewport_text.contains("BROWSER_FEATURE_UNSUPPORTED"),
        "{viewport_text}"
    );
    assert_eq!(driver.calls().len(), calls_before);

    let network = client
        .call_tool(
            CallToolRequestParams::new("browser_network")
                .with_arguments(arguments(serde_json::json!({}))),
        )
        .await
        .unwrap();
    let network_text = serde_json::to_string(&network).unwrap();
    assert!(!network_text.contains("Bearer secret"));
    assert!(!network_text.contains("session=secret"));

    let trace_stop = client
        .call_tool(
            CallToolRequestParams::new("browser_trace")
                .with_arguments(arguments(serde_json::json!({"action":"stop"}))),
        )
        .await
        .unwrap();
    assert!(!trace_stop.is_error.unwrap_or(false));
    assert!(serde_json::to_string(&trace_stop)
        .unwrap()
        .contains("artifact_id"));

    // Screenshot, upload, downloads, and save-download are structurally
    // unsupported under the lightpanda runtime and must fail closed without
    // any driver call, staging copy, or workspace write.
    let unsupported_calls_before = driver.calls().len();
    let screenshot = client
        .call_tool(
            CallToolRequestParams::new("browser_screenshot").with_arguments(arguments(
                serde_json::json!({"mode":"viewport","max_edge":2}),
            )),
        )
        .await
        .unwrap();
    let screenshot_text = serde_json::to_string(&screenshot).unwrap();
    assert!(screenshot.is_error.unwrap_or(false), "{screenshot_text}");
    assert!(
        screenshot_text.contains("BROWSER_FEATURE_UNSUPPORTED"),
        "{screenshot_text}"
    );

    let before_session = client
        .call_tool(CallToolRequestParams::new("browser_get_status"))
        .await
        .unwrap()
        .structured_content
        .unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let emulate = client.call_tool(CallToolRequestParams::new("browser_emulate_device").with_arguments(arguments(serde_json::json!({"emulation":{"source":"explicit","width":390,"height":844,"device_scale_factor":3.0,"is_mobile":true,"has_touch":true,"user_agent":"TestPhone/1.0","orientation":"portrait"}})))).await.unwrap();
    let emulate_text = serde_json::to_string(&emulate).unwrap();
    assert!(emulate.is_error.unwrap_or(false), "{emulate_text}");
    assert!(
        emulate_text.contains("BROWSER_FEATURE_UNSUPPORTED"),
        "{emulate_text}"
    );
    let after_session = client
        .call_tool(CallToolRequestParams::new("browser_get_status"))
        .await
        .unwrap()
        .structured_content
        .unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        before_session, after_session,
        "unsupported emulation must not restart the session"
    );

    let dialog = client
        .call_tool(
            CallToolRequestParams::new("browser_dialog")
                .with_arguments(arguments(serde_json::json!({"accept":true}))),
        )
        .await
        .unwrap();
    let dialog_text = serde_json::to_string(&dialog).unwrap();
    assert!(dialog.is_error.unwrap_or(false), "{dialog_text}");
    assert!(
        dialog_text.contains("BROWSER_FEATURE_UNSUPPORTED"),
        "{dialog_text}"
    );

    let upload = client
        .call_tool(
            CallToolRequestParams::new("browser_upload").with_arguments(arguments(
                serde_json::json!({"workspace_id":ws.id,"sources":[{"kind":"workspace","path":"upload.txt"}]}),
            )),
        )
        .await
        .unwrap();
    let upload_text = serde_json::to_string(&upload).unwrap();
    assert!(upload.is_error.unwrap_or(false), "{upload_text}");
    assert!(
        upload_text.contains("BROWSER_FEATURE_UNSUPPORTED"),
        "{upload_text}"
    );
    assert!(
        !active_artifact_dir(&dir).join("upload-staging").exists(),
        "unsupported upload created a staging directory"
    );

    std::fs::write(
        active_artifact_dir(&dir).join("result.txt"),
        b"download-body",
    )
    .unwrap();
    let downloads = client
        .call_tool(CallToolRequestParams::new("browser_downloads"))
        .await
        .unwrap();
    let downloads_text = serde_json::to_string(&downloads).unwrap();
    assert!(downloads.is_error.unwrap_or(false), "{downloads_text}");
    assert!(
        downloads_text.contains("BROWSER_FEATURE_UNSUPPORTED"),
        "{downloads_text}"
    );
    let save = client.call_tool(CallToolRequestParams::new("browser_save_download_to_workspace").with_arguments(arguments(serde_json::json!({"workspace_id":ws.id,"download_id":"d-0000000000000000-1","path":"saved/result.txt"})))).await.unwrap();
    let save_text = serde_json::to_string(&save).unwrap();
    assert!(save.is_error.unwrap_or(false), "{save_text}");
    assert!(
        save_text.contains("BROWSER_FEATURE_UNSUPPORTED"),
        "{save_text}"
    );
    assert!(
        !workspace_root.join("saved/result.txt").exists(),
        "unsupported save wrote into the workspace"
    );
    assert_eq!(
        driver.calls().len(),
        unsupported_calls_before,
        "unsupported tools reached the driver"
    );

    let steps = client.call_tool(CallToolRequestParams::new("browser_run_steps").with_arguments(arguments(serde_json::json!({"steps":[{"type":"navigate","url":"https://example.test/flow"},{"type":"snapshot"}]})))).await.unwrap();
    assert!(
        !steps.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&steps).unwrap()
    );
    assert_eq!(steps.structured_content.as_ref().unwrap()["ok"], true);

    let devtools = client
        .call_tool(CallToolRequestParams::new("browser_devtools"))
        .await
        .unwrap();
    assert!(devtools.is_error.unwrap_or(false));
    assert!(serde_json::to_string(&devtools)
        .unwrap()
        .contains("BROWSER_FEATURE_UNSUPPORTED"));

    let fill_audit = matching_audit(&core, "browser_fill");
    assert_eq!(fill_audit.len(), 1);
    assert!(
        !serde_json::to_string(fill_audit[0].detail.as_ref().unwrap())
            .unwrap()
            .contains("super-secret-value")
    );
}

#[tokio::test]
async fn disabled_allows_only_browser_get_status() {
    let settings = AppSettings {
        browser_automation_enabled: false,
        ..AppSettings::default()
    };
    let (port, _dir, core, driver, _browser) = spawn(settings).await;
    let client = client(port).await;
    let calls = [
        ("browser_open", serde_json::json!({})),
        ("browser_close", serde_json::json!({})),
        ("browser_tabs", serde_json::json!({"action":"list"})),
        ("browser_navigate", serde_json::json!({"action":"reload"})),
        ("browser_search", serde_json::json!({"query":"query"})),
        ("browser_snapshot", serde_json::json!({})),
        ("browser_click", serde_json::json!({"ref":"e1"})),
        (
            "browser_fill",
            serde_json::json!({"ref":"e1","text":"value"}),
        ),
        ("browser_press", serde_json::json!({"key":"Enter"})),
        (
            "browser_select",
            serde_json::json!({"ref":"e1","values":["x"]}),
        ),
        ("browser_hover", serde_json::json!({"ref":"e1"})),
        (
            "browser_drag",
            serde_json::json!({"start_ref":"e1","end_ref":"e2"}),
        ),
        (
            "browser_wait",
            serde_json::json!({"condition":{"kind":"delay","milliseconds":1}}),
        ),
        ("browser_dialog", serde_json::json!({"accept":true})),
        (
            "browser_set_viewport",
            serde_json::json!({"width":320,"height":568}),
        ),
        (
            "browser_emulate_device",
            serde_json::json!({"emulation":{"source":"explicit","width":320,"height":568,"device_scale_factor":1.0,"is_mobile":false,"has_touch":false,"user_agent":"test","orientation":"portrait"}}),
        ),
        ("browser_screenshot", serde_json::json!({"mode":"viewport"})),
        ("browser_console", serde_json::json!({})),
        ("browser_network", serde_json::json!({})),
        ("browser_dom", serde_json::json!({"ref":"e1"})),
        (
            "browser_evaluate",
            serde_json::json!({"expression":"() => 1"}),
        ),
        ("browser_performance", serde_json::json!({})),
        ("browser_trace", serde_json::json!({"action":"status"})),
        ("browser_devtools", serde_json::json!({})),
        (
            "browser_upload",
            serde_json::json!({"workspace_id":"missing","sources":[{"kind":"workspace","path":"x.txt"}]}),
        ),
        ("browser_downloads", serde_json::json!({})),
        (
            "browser_save_download_to_workspace",
            serde_json::json!({"workspace_id":"missing","download_id":"missing","path":"x.txt"}),
        ),
        (
            "browser_run_steps",
            serde_json::json!({"steps":[{"type":"snapshot"}]}),
        ),
    ];

    for (name, args) in calls {
        let result = client
            .call_tool(CallToolRequestParams::new(name).with_arguments(arguments(args)))
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{name}: {text}");
        assert!(
            text.contains("BROWSER_AUTOMATION_DISABLED"),
            "{name}: {text}"
        );
        let rows = matching_audit(&core, name);
        assert_eq!(rows.len(), 1, "{name} must produce exactly one audit row");
    }
    assert!(
        driver.calls().is_empty(),
        "disabled calls reached browser driver"
    );
}

#[tokio::test]
async fn lifecycle_disabled_status_is_readable_but_mutations_are_denied_once_and_audited_once() {
    let settings = AppSettings {
        browser_automation_enabled: false,
        ..AppSettings::default()
    };
    let (port, _dir, core, _driver, _browser) = spawn(settings).await;
    let client = client(port).await;

    let status = client
        .call_tool(CallToolRequestParams::new("browser_get_status"))
        .await
        .unwrap();
    let status_text = serde_json::to_string(&status).unwrap();
    assert!(!status.is_error.unwrap_or(false), "{status_text}");
    assert!(status_text.contains("\"enabled\":false"), "{status_text}");
    let rows = matching_audit(&core, "browser_get_status");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, "allow");
    assert_eq!(rows[0].detail.as_ref().unwrap()["enabled"], false);
    assert!(rows[0].detail.as_ref().unwrap().get("url").is_none());

    let open = client
        .call_tool(CallToolRequestParams::new("browser_open"))
        .await
        .unwrap();
    let open_text = serde_json::to_string(&open).unwrap();
    assert!(open.is_error.unwrap_or(false), "{open_text}");
    assert!(
        open_text.contains("BROWSER_AUTOMATION_DISABLED"),
        "{open_text}"
    );
    let rows = matching_audit(&core, "browser_open");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, "deny");
    assert_eq!(
        rows[0].error_code.as_deref(),
        Some("BROWSER_AUTOMATION_DISABLED")
    );

    let navigate = client
        .call_tool(
            CallToolRequestParams::new("browser_navigate").with_arguments(arguments(
                serde_json::json!({"action":"url","url":"https://example.com/"}),
            )),
        )
        .await
        .unwrap();
    assert!(navigate.is_error.unwrap_or(false));
    assert!(serde_json::to_string(&navigate)
        .unwrap()
        .contains("BROWSER_AUTOMATION_DISABLED"));
    assert_eq!(matching_audit(&core, "browser_navigate").len(), 1);
}

#[tokio::test]
async fn lifecycle_pause_wins_for_status_open_and_tabs() {
    let (port, _dir, core, _driver, _browser) = spawn(AppSettings::default()).await;
    core.pause.set(true);
    let client = client(port).await;

    for (name, params) in [
        ("browser_get_status", None),
        ("browser_open", None),
        ("browser_tabs", Some(serde_json::json!({"action":"list"}))),
    ] {
        let mut request = CallToolRequestParams::new(name);
        if let Some(value) = params {
            request = request.with_arguments(arguments(value));
        }
        let result = client.call_tool(request).await.unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{name}: {text}");
        assert!(text.contains("ACCESS_PAUSED"), "{name}: {text}");
        let rows = matching_audit(&core, name);
        assert_eq!(rows.len(), 1, "{name} must produce exactly one audit row");
        assert_eq!(rows[0].decision, "deny");
        assert_eq!(rows[0].error_code.as_deref(), Some("ACCESS_PAUSED"));
    }
}

#[tokio::test]
async fn navigation_url_is_validated_forwarded_and_audited_by_origin_only() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    client
        .call_tool(CallToolRequestParams::new("browser_open"))
        .await
        .unwrap();

    let secret_url = "https://example.com/a/private?secret=x#token";
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_navigate").with_arguments(arguments(
                serde_json::json!({"action":"url","url":secret_url}),
            )),
        )
        .await
        .unwrap();
    assert!(
        !result.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&result).unwrap()
    );

    // Native mapping: goto carries the full URL, and goto success is
    // verified with a follow-up getUrl and a bounded tree read.
    assert!(driver
        .calls()
        .iter()
        .any(|call| { call.tool == "goto" && call.arguments["url"] == secret_url }));
    let calls = driver.calls();
    let sequence: Vec<&str> = calls
        .iter()
        .filter(|call| ["goto", "getUrl", "tree"].contains(&call.tool.as_str()))
        .map(|call| call.tool.as_str())
        .collect();
    assert_eq!(sequence, vec!["goto", "getUrl", "tree"]);

    let rows = matching_audit(&core, "browser_navigate");
    assert_eq!(rows.len(), 1);
    let detail = rows[0].detail.as_ref().unwrap();
    assert_eq!(detail["origin"], "https://example.com");
    let serialized = serde_json::to_string(detail).unwrap();
    assert!(!serialized.contains("private"));
    assert!(!serialized.contains("secret=x"));
    assert!(!serialized.contains("token"));
}

#[tokio::test]
async fn navigation_rejects_file_url_before_driver_call() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let before = driver.calls().len();
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_navigate").with_arguments(arguments(
                serde_json::json!({"action":"url","url":"file:///C:/secret.txt"}),
            )),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("INVALID_ARGUMENT"), "{text}");
    assert_eq!(
        driver.calls().len(),
        before,
        "invalid URL reached the browser driver"
    );
    assert_eq!(matching_audit(&core, "browser_navigate").len(), 1);
}

#[tokio::test]
async fn navigation_search_uses_native_search_and_never_synthesizes_engine_urls() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    client
        .call_tool(CallToolRequestParams::new("browser_open"))
        .await
        .unwrap();

    let query = "rust tauri mcp & private";
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_search")
                .with_arguments(arguments(serde_json::json!({"query": query}))),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));

    // Native mapping: the query goes to the runtime's search tool verbatim.
    let search_call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|call| call.tool == "search")
        .expect("search must map to the native search tool");
    assert_eq!(search_call.arguments["query"], query);

    // No search-engine URL is ever synthesized on the host side.
    assert!(
        driver.calls().iter().all(|call| {
            call.arguments
                .get("url")
                .and_then(|v| v.as_str())
                .is_none_or(|url| !url.contains("google.com"))
        }),
        "search must never synthesize a search-engine url"
    );

    let rows = matching_audit(&core, "browser_search");
    assert_eq!(rows.len(), 1);
    let detail = rows[0].detail.as_ref().unwrap();
    assert_eq!(detail["query_bytes"], query.len());
    assert!(!serde_json::to_string(detail).unwrap().contains(query));
}

#[tokio::test]
async fn navigation_back_forward_reload_use_fixed_evaluate_scripts_and_invalidate_refs() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let snapshot = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snapshot.is_error.unwrap_or(false));

    for (action, script) in [
        ("back", "history.back()"),
        ("forward", "history.forward()"),
        ("reload", "location.reload()"),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("browser_navigate")
                    .with_arguments(arguments(serde_json::json!({"action": action}))),
            )
            .await
            .unwrap();
        assert!(
            !result.is_error.unwrap_or(false),
            "{action}: {}",
            serde_json::to_string(&result).unwrap()
        );

        // The fixed application-owned script ran, then the page was waited
        // to `load`, then the final URL was read.
        let eval = driver
            .calls()
            .into_iter()
            .rev()
            .find(|c| c.tool == "evaluate")
            .unwrap_or_else(|| panic!("{action} must run an evaluate script"));
        assert_eq!(eval.arguments["script"], script, "{action}");
        let wait = driver
            .calls()
            .into_iter()
            .rev()
            .find(|c| c.tool == "waitForState")
            .unwrap_or_else(|| panic!("{action} must wait for load"));
        assert_eq!(wait.arguments["state"], "load", "{action}");
        assert!(
            driver.calls().iter().any(|c| c.tool == "getUrl"),
            "{action} must read the final url"
        );

        // History navigation replaces the DOM: refs are stale until a new
        // snapshot.
        let click = client
            .call_tool(
                CallToolRequestParams::new("browser_click")
                    .with_arguments(arguments(serde_json::json!({"ref":"e4"}))),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&click).unwrap();
        assert!(click.is_error.unwrap_or(false), "{action}: {text}");
        assert!(text.contains("BROWSER_STALE_REF"), "{action}: {text}");

        let resnap = client
            .call_tool(CallToolRequestParams::new("browser_snapshot"))
            .await
            .unwrap();
        assert!(!resnap.is_error.unwrap_or(false), "{action}");
    }
}

#[tokio::test]
async fn navigation_goto_success_without_verification_is_a_navigation_error() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    // Variant 1: getUrl comes back empty -> the navigation is unverified.
    driver.respond("getUrl", String::new());
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_navigate").with_arguments(arguments(
                serde_json::json!({"action":"url","url":"https://example.com/"}),
            )),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_PROTOCOL_ERROR"), "{text}");
    assert!(text.contains("could not be verified"), "{text}");
    assert!(
        driver.calls().iter().any(|c| c.tool == "goto"),
        "goto must run before verification"
    );
    assert!(
        driver.calls().iter().all(|c| c.tool != "tree"),
        "unverified navigation must stop before the tree read"
    );

    // Variant 2: the bounded tree read comes back empty -> unverified.
    driver.respond("tree", String::new());
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_navigate").with_arguments(arguments(
                serde_json::json!({"action":"url","url":"https://example.com/"}),
            )),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("could not be verified"), "{text}");
}

#[tokio::test]
async fn interaction_press_and_evaluate_map_to_native_tools() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let snapshot = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snapshot.is_error.unwrap_or(false));

    // Plain expression passes through; function literals are invoked once.
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_evaluate").with_arguments(arguments(
                serde_json::json!({"expression":"() => document.title"}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let eval = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "evaluate")
        .expect("evaluate must map to the native evaluate tool");
    assert_eq!(eval.arguments["script"], "(() => document.title)()");

    // With a ref, the selector is recovered through nodeDetails and bound
    // as the element argument.
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_evaluate").with_arguments(arguments(
                serde_json::json!({"expression":"(el) => el.textContent","ref":"e2"}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    assert!(driver
        .calls()
        .iter()
        .any(|c| c.tool == "nodeDetails" && c.arguments["backendNodeId"] == 2));
    let eval = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "evaluate")
        .unwrap();
    assert_eq!(
        eval.arguments["script"],
        "((el) => el.textContent)(document.querySelector(\"#nd-2\"))"
    );

    // Audit stores byte counts only, never the expression.
    let rows = matching_audit(&core, "browser_evaluate");
    assert_eq!(rows.len(), 2);
    for row in &rows {
        let detail = row.detail.as_ref().unwrap();
        let serialized = serde_json::to_string(detail).unwrap();
        assert!(!serialized.contains("document.title"), "{serialized}");
        assert!(!serialized.contains("textContent"), "{serialized}");
    }

    // press maps to the native press tool with the key verbatim.
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_press")
                .with_arguments(arguments(serde_json::json!({"key":"Enter"}))),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let press = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "press")
        .expect("press must map to the native press tool");
    assert_eq!(press.arguments["key"], "Enter");
}

#[tokio::test]
async fn wait_text_and_url_conditions_map_to_fixed_waitforscript_with_encoded_data() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    let text = "Hello \"wide\" world";
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_wait").with_arguments(arguments(
                serde_json::json!({"condition":{"kind":"text_present","text":text},"timeout_ms":1500}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let wait = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "waitForScript")
        .expect("text_present must map to waitForScript");
    let encoded = serde_json::to_string(text).unwrap();
    assert_eq!(
        wait.arguments["script"],
        format!("document.body && document.body.innerText.includes({encoded})")
    );
    assert_eq!(wait.arguments["timeout"], 1500);

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_wait").with_arguments(arguments(
                serde_json::json!({"condition":{"kind":"text_absent","text":"bye"},"timeout_ms":1500}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let wait = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "waitForScript")
        .unwrap();
    assert_eq!(
        wait.arguments["script"],
        "!document.body || !document.body.innerText.includes(\"bye\")"
    );

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_wait").with_arguments(arguments(
                serde_json::json!({"condition":{"kind":"url_contains","value":"example"},"timeout_ms":1500}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let wait = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "waitForScript")
        .unwrap();
    assert_eq!(
        wait.arguments["script"],
        "location.href.includes(\"example\")"
    );
}

#[tokio::test]
async fn wait_ref_visible_recovers_selector_and_waits_for_selector() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let snapshot = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snapshot.is_error.unwrap_or(false));

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_wait").with_arguments(arguments(
                serde_json::json!({"condition":{"kind":"ref_visible","ref_id":"e4"},"timeout_ms":2500}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    assert!(driver
        .calls()
        .iter()
        .any(|c| c.tool == "nodeDetails" && c.arguments["backendNodeId"] == 4));
    let wait = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "waitForSelector")
        .expect("ref_visible must map to waitForSelector");
    assert_eq!(wait.arguments["selector"], "#nd-4");
    assert_eq!(wait.arguments["timeout"], 2500);
}

#[tokio::test]
async fn wait_ref_hidden_waits_for_node_gone_and_stale_ref_succeeds_immediately() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let snapshot = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snapshot.is_error.unwrap_or(false));

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_wait").with_arguments(arguments(
                serde_json::json!({"condition":{"kind":"ref_hidden","ref_id":"e4"},"timeout_ms":2500}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let wait = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "waitForScript")
        .expect("ref_hidden must map to waitForScript");
    assert_eq!(
        wait.arguments["script"],
        "document.querySelector(\"#nd-4\") === null"
    );

    // Navigate away: every ref becomes stale. A hidden wait on a stale ref
    // succeeds immediately because the node is already gone.
    let nav = client
        .call_tool(
            CallToolRequestParams::new("browser_navigate").with_arguments(arguments(
                serde_json::json!({"action":"url","url":"https://example.com/next"}),
            )),
        )
        .await
        .unwrap();
    assert!(!nav.is_error.unwrap_or(false));
    let waits_before = driver
        .calls()
        .iter()
        .filter(|c| c.tool == "waitForScript")
        .count();
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_wait").with_arguments(arguments(
                serde_json::json!({"condition":{"kind":"ref_hidden","ref_id":"e4"},"timeout_ms":2500}),
            )),
        )
        .await
        .unwrap();
    assert!(
        !result.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&result).unwrap()
    );
    let waits_after = driver
        .calls()
        .iter()
        .filter(|c| c.tool == "waitForScript")
        .count();
    assert_eq!(
        waits_before, waits_after,
        "stale ref hidden wait must not call the page"
    );
}

#[tokio::test]
async fn wait_load_state_maps_each_public_state_to_native_waits() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_wait").with_arguments(arguments(
                serde_json::json!({"condition":{"kind":"load_state","state":"complete"},"timeout_ms":3000}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let wait = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "waitForState")
        .expect("complete must map to waitForState");
    assert_eq!(wait.arguments["state"], "load");
    assert_eq!(wait.arguments["timeout"], 3000);
    let verify = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "waitForScript")
        .expect("complete must verify readyState");
    assert_eq!(
        verify.arguments["script"],
        "document.readyState === \"complete\""
    );

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_wait").with_arguments(arguments(
                serde_json::json!({"condition":{"kind":"load_state","state":"loading"},"timeout_ms":3000}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let wait = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "waitForScript")
        .unwrap();
    assert_eq!(
        wait.arguments["script"],
        "document.readyState === \"loading\""
    );

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_wait").with_arguments(arguments(
                serde_json::json!({"condition":{"kind":"load_state","state":"interactive"},"timeout_ms":3000}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let wait = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "waitForScript")
        .unwrap();
    assert_eq!(
        wait.arguments["script"],
        "document.readyState === \"interactive\""
    );
}

#[tokio::test]
async fn wait_delay_sleeps_on_host_without_upstream_calls_and_is_bounded() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let before = driver.calls().len();

    let started = std::time::Instant::now();
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_wait").with_arguments(arguments(
                serde_json::json!({"condition":{"kind":"delay","milliseconds":150},"timeout_ms":2000}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(120),
        "delay must actually wait on the host"
    );
    assert_eq!(
        driver.calls().len(),
        before,
        "delay must not call the page child"
    );

    for bad in [
        serde_json::json!({"condition":{"kind":"delay","milliseconds":3000},"timeout_ms":2000}),
        serde_json::json!({"condition":{"kind":"delay","milliseconds":0},"timeout_ms":2000}),
    ] {
        let result = client
            .call_tool(CallToolRequestParams::new("browser_wait").with_arguments(arguments(bad)))
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{text}");
        assert!(text.contains("INVALID_ARGUMENT"), "{text}");
    }
    assert_eq!(driver.calls().len(), before);
}

#[tokio::test]
async fn wait_delay_is_interrupted_by_pause() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let before = driver.calls().len();

    let started = std::time::Instant::now();
    let handle = tokio::spawn(async move {
        client
            .call_tool(
                CallToolRequestParams::new("browser_wait").with_arguments(arguments(
                    serde_json::json!({"condition":{"kind":"delay","milliseconds":5000},"timeout_ms":10000}),
                )),
            )
            .await
            .unwrap()
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    core.pause.set(true);
    let result = handle.await.unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("ACCESS_PAUSED"), "{text}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "pause must interrupt the delay promptly"
    );
    assert_eq!(driver.calls().len(), before);
}

#[tokio::test]
async fn process_lost_marks_only_that_page_lost_and_never_replays_side_effects() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let snapshot = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snapshot.is_error.unwrap_or(false));

    driver.fail(
        "click",
        CoreError::new(ErrorCode::BrowserProcessLost, "page process was lost"),
    );
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_click")
                .with_arguments(arguments(serde_json::json!({"ref":"e4"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_PROCESS_LOST"), "{text}");

    // The side-effecting action reached the child exactly once: no replay.
    let clicks = driver.calls().iter().filter(|c| c.tool == "click").count();
    assert_eq!(clicks, 1, "a lost action must never be replayed");
    let rows = matching_audit(&core, "browser_click");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, "error");
    assert_eq!(rows[0].error_code.as_deref(), Some("BROWSER_PROCESS_LOST"));

    // The page is latched lost: further calls fail closed without reaching
    // the driver.
    let calls_before = driver.calls().len();
    let snap = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    let snap_text = serde_json::to_string(&snap).unwrap();
    assert!(snap.is_error.unwrap_or(false), "{snap_text}");
    assert!(snap_text.contains("BROWSER_PROCESS_LOST"), "{snap_text}");
    assert_eq!(
        driver.calls().len(),
        calls_before,
        "a lost page must never receive another call"
    );

    // Sibling isolation: a new logical tab gets a fresh child and works.
    let new_tab = client
        .call_tool(
            CallToolRequestParams::new("browser_tabs")
                .with_arguments(arguments(serde_json::json!({"action":"new"}))),
        )
        .await
        .unwrap();
    assert!(!new_tab.is_error.unwrap_or(false));
    let snap = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(
        !snap.is_error.unwrap_or(false),
        "sibling page must survive one lost page: {}",
        serde_json::to_string(&snap).unwrap()
    );
}

#[tokio::test]
async fn lifecycle_tabs_are_manager_owned_with_one_child_per_page() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let opened = client
        .call_tool(CallToolRequestParams::new("browser_open"))
        .await
        .unwrap();
    assert!(!opened.is_error.unwrap_or(false));
    assert_eq!(
        driver.starts.load(Ordering::SeqCst),
        1,
        "open spawns one child"
    );

    let new_tab = client
        .call_tool(
            CallToolRequestParams::new("browser_tabs").with_arguments(arguments(
                serde_json::json!({"action":"new","url":"https://example.org/new"}),
            )),
        )
        .await
        .unwrap();
    assert_eq!(
        driver.starts.load(Ordering::SeqCst),
        2,
        "tabs new spawns a second child"
    );
    let pages = new_tab
        .structured_content
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(pages.len(), 2);
    let first_id = pages[0]["page_id"].as_str().unwrap().to_string();
    let second_id = pages[1]["page_id"].as_str().unwrap().to_string();
    assert!(first_id.starts_with("p-"));
    assert!(second_id.starts_with("p-"));
    assert_ne!(first_id, second_id);
    assert!(
        pages[1]["active"].as_bool().unwrap(),
        "new page becomes active"
    );

    // The new page was navigated with the native goto primitive.
    let goto = driver
        .calls()
        .into_iter()
        .rev()
        .find(|call| call.tool == "goto")
        .expect("tabs new must navigate via goto");
    assert_eq!(goto.arguments["url"], "https://example.org/new");

    // Logical tabs never call an upstream tabs primitive.
    for call in driver.calls() {
        assert_ne!(call.tool, "browser_tabs", "tabs must be manager-owned");
    }

    // Select does not spawn or restart.
    let select = client
        .call_tool(
            CallToolRequestParams::new("browser_tabs").with_arguments(arguments(
                serde_json::json!({"action":"select","page_id":first_id.clone()}),
            )),
        )
        .await
        .unwrap();
    assert!(!select.is_error.unwrap_or(false));
    assert_eq!(driver.starts.load(Ordering::SeqCst), 2);
    assert_eq!(driver.stops.load(Ordering::SeqCst), 0);

    let close = client
        .call_tool(
            CallToolRequestParams::new("browser_tabs").with_arguments(arguments(
                serde_json::json!({"action":"close","page_id":first_id.clone()}),
            )),
        )
        .await
        .unwrap();
    assert!(!close.is_error.unwrap_or(false));
    assert_eq!(
        driver.stops.load(Ordering::SeqCst),
        1,
        "close stops only that child"
    );

    let pages = close
        .structured_content
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0]["page_id"], second_id);
    assert!(pages[0]["active"].as_bool().unwrap());

    let stale = client
        .call_tool(
            CallToolRequestParams::new("browser_tabs").with_arguments(arguments(
                serde_json::json!({"action":"select","page_id":first_id}),
            )),
        )
        .await
        .unwrap();
    let stale_text = serde_json::to_string(&stale).unwrap();
    assert!(stale.is_error.unwrap_or(false), "{stale_text}");
    assert!(
        stale_text.contains("BROWSER_TARGET_NOT_FOUND"),
        "{stale_text}"
    );
}

async fn open_browser(client: &rmcp::service::RunningService<rmcp::service::RoleClient, ()>) {
    let opened = client
        .call_tool(CallToolRequestParams::new("browser_open"))
        .await
        .unwrap();
    assert!(
        !opened.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&opened).unwrap()
    );
}

#[tokio::test]
async fn interaction_tools_resolve_refs_to_native_selectors_via_node_details() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    // Snapshot registers opaque refs e1..e4 from the tree backend node ids.
    let snap = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snap.is_error.unwrap_or(false));

    // click: ref -> nodeDetails(backendNodeId) -> native click(selector).
    let click = client
        .call_tool(
            CallToolRequestParams::new("browser_click")
                .with_arguments(arguments(serde_json::json!({"ref":"e4"}))),
        )
        .await
        .unwrap();
    assert!(
        !click.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&click).unwrap()
    );
    let details = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "nodeDetails")
        .expect("click must recover a selector via nodeDetails");
    assert_eq!(details.arguments, serde_json::json!({"backendNodeId": 4}));
    let click_call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "click")
        .expect("click must reach the native click tool");
    assert_eq!(
        click_call.arguments,
        serde_json::json!({"selector": "#nd-4"})
    );

    // Side-effecting interactions invalidate refs; re-snapshot before the next.
    let resnap = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!resnap.is_error.unwrap_or(false));
    let fill = client
        .call_tool(
            CallToolRequestParams::new("browser_fill")
                .with_arguments(arguments(serde_json::json!({"ref":"e2","text":"secret"}))),
        )
        .await
        .unwrap();
    assert!(
        !fill.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&fill).unwrap()
    );
    let fill_call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "fill")
        .expect("fill must reach the native fill tool");
    assert_eq!(
        fill_call.arguments,
        serde_json::json!({"selector": "#nd-2", "value": "secret"})
    );

    client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    let select = client
        .call_tool(
            CallToolRequestParams::new("browser_select").with_arguments(arguments(
                serde_json::json!({"ref":"e3","values":["US","CA"]}),
            )),
        )
        .await
        .unwrap();
    assert!(
        !select.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&select).unwrap()
    );
    // Lightpanda selectOption accepts one value per call; the selector is
    // recovered once and reused for every value.
    let select_calls: Vec<RecordedCall> = driver
        .calls()
        .into_iter()
        .filter(|c| c.tool == "selectOption")
        .collect();
    assert_eq!(select_calls.len(), 2);
    assert_eq!(
        select_calls[0].arguments,
        serde_json::json!({"selector": "#nd-3", "value": "US"})
    );
    assert_eq!(
        select_calls[1].arguments,
        serde_json::json!({"selector": "#nd-3", "value": "CA"})
    );

    client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    let hover = client
        .call_tool(
            CallToolRequestParams::new("browser_hover")
                .with_arguments(arguments(serde_json::json!({"ref":"e1"}))),
        )
        .await
        .unwrap();
    assert!(
        !hover.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&hover).unwrap()
    );
    // Lightpanda has no native hover primitive: hover is a synthetic
    // mouseover sequence bound to the recovered selector as data.
    let hover_call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "evaluate")
        .expect("hover must route through evaluate");
    let hover_script = hover_call.arguments["script"].as_str().unwrap();
    assert!(hover_script.contains("mouseover"), "{hover_script}");
    assert!(
        hover_script.ends_with(")(\"#nd-1\")"),
        "hover must bind the recovered selector as data: {hover_script}"
    );

    // Neither opaque refs nor backend node ids may reach action arguments.
    for call in driver.calls() {
        if matches!(
            call.tool.as_str(),
            "click" | "hover" | "fill" | "selectOption"
        ) {
            let text = serde_json::to_string(&call.arguments).unwrap();
            assert!(
                !text.contains("backendNodeId"),
                "{} leaks a backend node id: {text}",
                call.tool
            );
            for leaked in ["\"e1\"", "\"e2\"", "\"e3\"", "\"e4\""] {
                assert!(
                    !text.contains(leaked),
                    "{} leaks a public ref: {text}",
                    call.tool
                );
            }
        }
    }

    let rows = matching_audit(&core, "browser_fill");
    assert_eq!(rows.len(), 1);
    let detail = rows[0].detail.as_ref().unwrap();
    assert_eq!(detail["target"], "e2");
    assert_eq!(detail["text_bytes"], 6);
    assert!(
        !serde_json::to_string(detail).unwrap().contains("secret"),
        "fill audit must store byte counts only"
    );
}

#[tokio::test]
async fn ref_ids_in_snapshot_are_opaque_and_backend_ids_never_leak() {
    let (port, _dir, _core, _driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    let result = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let snapshot = result.structured_content.as_ref().unwrap()["snapshot"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        snapshot.contains("e1 WebArea 'Browser Test App'"),
        "{snapshot}"
    );
    assert!(snapshot.contains("e2 [i] textbox 'Name'"), "{snapshot}");
    assert!(snapshot.contains("e4 [i] button 'Submit'"), "{snapshot}");
    // Raw backend node ids must not survive as line identities: every
    // snapshot line starts with an opaque ref, never a bare digit.
    for line in snapshot.lines() {
        let rest = line.trim_start_matches(' ');
        assert!(
            !rest.chars().next().is_some_and(|c| c.is_ascii_digit()),
            "raw backend id leaked in snapshot line: {line}"
        );
    }

    // A second snapshot in the same generation reuses the same refs.
    let again = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert_eq!(
        again.structured_content.as_ref().unwrap()["snapshot"]
            .as_str()
            .unwrap(),
        snapshot
    );
}

#[tokio::test]
async fn ref_is_scoped_to_its_page_and_cross_page_use_fails_closed() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    // New page; snapshot registers refs on that page only.
    let new_tab = client
        .call_tool(
            CallToolRequestParams::new("browser_tabs")
                .with_arguments(arguments(serde_json::json!({"action":"new"}))),
        )
        .await
        .unwrap();
    let pages = new_tab
        .structured_content
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    let first_id = pages[0]["page_id"].as_str().unwrap().to_string();
    let second_id = pages[1]["page_id"].as_str().unwrap().to_string();
    let snap = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snap.is_error.unwrap_or(false));

    // On the first page no ref was ever registered: fail closed.
    client
        .call_tool(
            CallToolRequestParams::new("browser_tabs").with_arguments(arguments(
                serde_json::json!({"action":"select","page_id":first_id}),
            )),
        )
        .await
        .unwrap();
    let cross = client
        .call_tool(
            CallToolRequestParams::new("browser_click")
                .with_arguments(arguments(serde_json::json!({"ref":"e1"}))),
        )
        .await
        .unwrap();
    let cross_text = serde_json::to_string(&cross).unwrap();
    assert!(cross.is_error.unwrap_or(false), "{cross_text}");
    assert!(cross_text.contains("BROWSER_STALE_REF"), "{cross_text}");

    // Back on the second page the same ref text resolves normally.
    client
        .call_tool(
            CallToolRequestParams::new("browser_tabs").with_arguments(arguments(
                serde_json::json!({"action":"select","page_id":second_id}),
            )),
        )
        .await
        .unwrap();
    let ok = client
        .call_tool(
            CallToolRequestParams::new("browser_click")
                .with_arguments(arguments(serde_json::json!({"ref":"e4"}))),
        )
        .await
        .unwrap();
    assert!(
        !ok.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&ok).unwrap()
    );
    let click_call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "click")
        .expect("native click");
    assert_eq!(
        click_call.arguments,
        serde_json::json!({"selector": "#nd-4"})
    );
}

#[tokio::test]
async fn ref_malformed_ids_are_rejected_before_any_upstream_call() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();

    for bad in ["", "1", "x1", "e1x", "E1", "e 1"] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("browser_click")
                    .with_arguments(arguments(serde_json::json!({"ref": bad}))),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{bad:?}: {text}");
        assert!(text.contains("INVALID_ARGUMENT"), "{bad:?}: {text}");
    }
    assert!(
        driver
            .calls()
            .iter()
            .all(|c| c.tool != "nodeDetails" && c.tool != "click"),
        "malformed refs must be rejected before any upstream call"
    );
}

#[tokio::test]
async fn interaction_stale_ref_surfaces_stable_code_and_audits_error() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    // Register refs, then navigate: navigation invalidates the generation.
    client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    let nav = client
        .call_tool(
            CallToolRequestParams::new("browser_open").with_arguments(arguments(
                serde_json::json!({"url":"https://example.test/next"}),
            )),
        )
        .await
        .unwrap();
    assert!(!nav.is_error.unwrap_or(false));

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_click")
                .with_arguments(arguments(serde_json::json!({"ref":"e4"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_STALE_REF"), "{text}");

    // Stale refs fail closed before any native action.
    assert!(
        driver
            .calls()
            .iter()
            .all(|c| c.tool != "nodeDetails" && c.tool != "click"),
        "stale ref reached a native action"
    );

    let rows = matching_audit(&core, "browser_click");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, "error");
    assert_eq!(rows[0].error_code.as_deref(), Some("BROWSER_STALE_REF"));
}

#[tokio::test]
async fn snapshot_300kib_is_truncated_to_256kib_with_byte_only_audit() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    driver.respond("tree", "x".repeat(300 * 1024));

    let result = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let structured = result.structured_content.as_ref().unwrap().clone();
    let snapshot = structured["snapshot"].as_str().unwrap();
    assert!(
        snapshot.len() <= 256 * 1024,
        "snapshot must be bounded to 256 KiB, got {}",
        snapshot.len()
    );
    assert_eq!(structured["truncated"], true);

    let rows = matching_audit(&core, "browser_snapshot");
    assert_eq!(rows.len(), 1);
    let detail = rows[0].detail.as_ref().unwrap();
    assert_eq!(detail["returned_bytes"], serde_json::json!(snapshot.len()));
    assert_eq!(detail["truncated"], true);
    assert!(detail.get("snapshot").is_none());
    assert!(
        !serde_json::to_string(detail).unwrap().contains("xxx"),
        "snapshot audit must record byte counts only"
    );
}

#[tokio::test]
async fn viewport_is_unsupported_and_invalid_sizes_are_rejected_before_any_effect() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let calls_before = driver.calls().len();

    for (width, height) in [(320u32, 568u32), (375, 812), (768, 1024), (1440, 900)] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("browser_set_viewport").with_arguments(arguments(
                    serde_json::json!({"width": width, "height": height}),
                )),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{width}x{height}: {text}");
        assert!(text.contains("BROWSER_FEATURE_UNSUPPORTED"), "{text}");
    }
    assert_eq!(
        driver.calls().len(),
        calls_before,
        "unsupported viewport must have zero upstream side effects"
    );

    for (width, height) in [(0u32, 600u32), (9000, 600)] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("browser_set_viewport").with_arguments(arguments(
                    serde_json::json!({"width": width, "height": height}),
                )),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{width}x{height}: {text}");
        assert!(text.contains("BROWSER_INVALID_VIEWPORT"), "{text}");
    }
    assert_eq!(
        driver.calls().len(),
        calls_before,
        "invalid viewport reached the driver"
    );
}

#[tokio::test]
async fn emulate_device_fails_closed_with_zero_side_effects() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let starts_before = driver.starts.load(Ordering::SeqCst);
    let stops_before = driver.stops.load(Ordering::SeqCst);
    let calls_before = driver.calls().len();

    let emulate = client
        .call_tool(
            CallToolRequestParams::new("browser_emulate_device").with_arguments(arguments(
                serde_json::json!({"emulation":{"source":"preset","name":"Test Phone"}}),
            )),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&emulate).unwrap();
    assert!(emulate.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_FEATURE_UNSUPPORTED"), "{text}");

    assert_eq!(
        driver.starts.load(Ordering::SeqCst),
        starts_before,
        "unsupported emulation must not restart anything"
    );
    assert_eq!(
        driver.stops.load(Ordering::SeqCst),
        stops_before,
        "unsupported emulation must not stop anything"
    );
    assert_eq!(
        driver.calls().len(),
        calls_before,
        "unsupported emulation must have zero upstream side effects"
    );
}

#[tokio::test]
async fn unsupported_screenshot_fails_closed_with_zero_driver_calls() {
    // OS Screen Capture state is irrelevant: the Lightpanda runtime has no
    // screenshot primitive, so every mode fails closed before any driver call.
    let settings = AppSettings {
        screen_capture_enabled: false,
        ..AppSettings::default()
    };
    let (port, _dir, core, driver, _browser) = spawn(settings).await;
    let client = client(port).await;
    open_browser(&client).await;

    for args in [
        serde_json::json!({"mode":"viewport"}),
        serde_json::json!({"mode":"full_page"}),
        serde_json::json!({"mode":"element","ref":"e1"}),
        serde_json::json!({"mode":"element"}),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("browser_screenshot")
                    .with_arguments(arguments(args.clone())),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{args}: {text}");
        assert!(
            text.contains("BROWSER_FEATURE_UNSUPPORTED"),
            "{args}: {text}"
        );
    }
    assert!(
        driver.calls().is_empty(),
        "unsupported screenshot must not reach the driver: {:?}",
        driver.calls()
    );
    let rows = matching_audit(&core, "browser_screenshot");
    assert_eq!(rows.len(), 4);
    for row in &rows {
        assert_eq!(row.decision, "error");
    }
}

#[tokio::test]
async fn console_cursor_levels_and_audit_metadata() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    // Lightpanda 0.3.7 `consoleLogs` shape: `[tag] text` lines. Each call
    // drains the child buffer, so the app accumulates drained batches into a
    // stable ring and paginates over it.
    driver.respond(
        "consoleLogs",
        "[info] one\n[warning] two\n[error] three".to_string(),
    );
    driver.respond("consoleLogs", "[info] four\n[debug] five".to_string());

    let page1 = client
        .call_tool(
            CallToolRequestParams::new("browser_console")
                .with_arguments(arguments(serde_json::json!({"limit":2}))),
        )
        .await
        .unwrap();
    assert!(!page1.is_error.unwrap_or(false));
    let structured = page1.structured_content.as_ref().unwrap().clone();
    assert_eq!(structured["count"], 2);
    assert_eq!(structured["messages"], "[info] one\n[warning] two");
    assert_eq!(structured["levels"]["info"], 1);
    assert_eq!(structured["levels"]["warning"], 1);
    let cursor1 = structured["next_cursor"].as_str().unwrap().to_string();

    let page2 = client
        .call_tool(
            CallToolRequestParams::new("browser_console")
                .with_arguments(arguments(serde_json::json!({"limit":2,"cursor":cursor1}))),
        )
        .await
        .unwrap();
    let structured = page2.structured_content.as_ref().unwrap().clone();
    // The default level is info, so the debug line stays in the ring but is
    // filtered out of every page: page2 is already the last page.
    assert_eq!(structured["messages"], "[error] three\n[info] four");
    assert!(
        structured["next_cursor"].is_null(),
        "filtered debug lines must not extend info-level pagination"
    );

    // Audit stores only count/levels/cursor, never message text.
    let rows = matching_audit(&core, "browser_console");
    assert_eq!(rows.len(), 2);
    for row in &rows {
        let detail = row.detail.as_ref().unwrap();
        let keys: std::collections::BTreeSet<String> =
            detail.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            ["count", "cursor", "levels"]
                .into_iter()
                .map(str::to_string)
                .collect::<std::collections::BTreeSet<String>>(),
            "console audit must store count/levels/cursor only"
        );
        assert!(
            !serde_json::to_string(detail).unwrap().contains("one"),
            "console audit leaks message text"
        );
    }

    let bad = client
        .call_tool(
            CallToolRequestParams::new("browser_console")
                .with_arguments(arguments(serde_json::json!({"cursor":"not-a-cursor"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&bad).unwrap();
    assert!(bad.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("INVALID_ARGUMENT"), "{text}");

    for args in [
        serde_json::json!({"level":"verbose"}),
        serde_json::json!({"limit":0}),
        serde_json::json!({"limit":501}),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("browser_console")
                    .with_arguments(arguments(args.clone())),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{args}: {text}");
        assert!(text.contains("INVALID_ARGUMENT"), "{args}: {text}");
    }

    // Level filtering happens app-side over the ring; the native consoleLogs
    // drain call takes no level/limit arguments.
    let debug = client
        .call_tool(
            CallToolRequestParams::new("browser_console")
                .with_arguments(arguments(serde_json::json!({"level":"debug","limit":100}))),
        )
        .await
        .unwrap();
    let structured = debug.structured_content.as_ref().unwrap().clone();
    assert_eq!(
        structured["messages"],
        "[info] one\n[warning] two\n[error] three\n[info] four\n[debug] five"
    );
    assert_eq!(structured["levels"]["debug"], 1);

    let errors = client
        .call_tool(
            CallToolRequestParams::new("browser_console")
                .with_arguments(arguments(serde_json::json!({"level":"error","limit":100}))),
        )
        .await
        .unwrap();
    let structured = errors.structured_content.as_ref().unwrap().clone();
    assert_eq!(structured["messages"], "[error] three");
    assert_eq!(structured["levels"]["error"], 1);

    let console_calls = driver
        .calls()
        .into_iter()
        .filter(|c| c.tool == "consoleLogs")
        .collect::<Vec<_>>();
    assert!(
        !console_calls.is_empty(),
        "console must drain the native consoleLogs tool"
    );
    for call in &console_calls {
        assert!(
            call.arguments.as_object().unwrap().is_empty(),
            "consoleLogs drain takes no arguments: {:?}",
            call.arguments
        );
    }
}

#[tokio::test]
async fn network_cursor_filter_and_redaction() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    // Lightpanda exposes network activity only through Performance Resource
    // Timing: name/initiatorType/duration/transferSize. No method, status, or
    // headers exist to fabricate. The sensitive query value stays covered by
    // app-side redaction.
    let resources = "{\"resources\":[{\"name\":\"https://a.test/api/items?token=abc\",\"initiatorType\":\"fetch\",\"duration\":10.4,\"transferSize\":200},{\"name\":\"https://b.test/logo.png\",\"initiatorType\":\"img\",\"duration\":3,\"transferSize\":1024},{\"name\":\"https://c.test/api/users\",\"initiatorType\":\"xmlhttprequest\",\"duration\":7,\"transferSize\":512}]}";
    driver.respond("evaluate", resources.to_string());
    driver.respond("evaluate", resources.to_string());

    let page1 = client
        .call_tool(
            CallToolRequestParams::new("browser_network").with_arguments(arguments(
                serde_json::json!({"limit":1,"filter":"api","include_static":true}),
            )),
        )
        .await
        .unwrap();
    assert!(!page1.is_error.unwrap_or(false));
    let structured = page1.structured_content.as_ref().unwrap().clone();
    assert_eq!(structured["count"], 1);
    let requests = structured["requests"].as_str().unwrap();
    assert!(
        requests.contains(
            "[fetch] https://a.test/api/items?token=[redacted] => duration=10ms size=200"
        ),
        "{requests}"
    );
    assert!(!requests.contains("token=abc"), "query secret leaked");
    assert!(
        structured["coverage"]
            .as_str()
            .unwrap()
            .contains("method, status, and headers are not available"),
        "coverage must state what this runtime cannot report"
    );
    let cursor = structured["next_cursor"].as_str().unwrap().to_string();

    let page2 = client
        .call_tool(
            CallToolRequestParams::new("browser_network")
                .with_arguments(arguments(serde_json::json!({"limit":10,"cursor":cursor,"filter":"api","include_static":true}))),
        )
        .await
        .unwrap();
    let structured = page2.structured_content.as_ref().unwrap().clone();
    let requests = structured["requests"].as_str().unwrap();
    assert!(
        requests.contains("[xmlhttprequest] https://c.test/api/users => duration=7ms size=512"),
        "{requests}"
    );
    assert!(structured["next_cursor"].is_null());

    // The filter is compiled app-side; the native call is a fixed Resource
    // Timing script and never carries the filter.
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "evaluate")
        .unwrap();
    let script = call.arguments["script"].as_str().unwrap();
    assert!(
        script.contains("performance.getEntriesByType('resource')"),
        "{script}"
    );
    assert!(!script.contains("api"), "filter must not reach the page");

    // include_static=false drops static subresources by initiatorType.
    driver.respond("evaluate", resources.to_string());
    let no_static = client
        .call_tool(
            CallToolRequestParams::new("browser_network")
                .with_arguments(arguments(serde_json::json!({"include_static":false}))),
        )
        .await
        .unwrap();
    let structured = no_static.structured_content.as_ref().unwrap().clone();
    let requests = structured["requests"].as_str().unwrap();
    assert!(!requests.contains("logo.png"), "{requests}");
    assert_eq!(structured["count"], 2);

    let rows = matching_audit(&core, "browser_network");
    assert_eq!(rows.len(), 3);
    for row in &rows {
        let detail = row.detail.as_ref().unwrap();
        let keys: std::collections::BTreeSet<String> =
            detail.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            ["count", "cursor"]
                .into_iter()
                .map(str::to_string)
                .collect::<std::collections::BTreeSet<String>>(),
            "network audit must store count/cursor only"
        );
    }

    // The filter is a regular expression; length/control-character/regex
    // guards reject bad filters before any upstream call is made.
    let upstream_calls = driver
        .calls()
        .iter()
        .filter(|c| c.tool == "evaluate")
        .count();
    for bad in [
        serde_json::json!({"filter":"a\nb"}),
        serde_json::json!({"filter":"x".repeat(4097)}),
        serde_json::json!({"filter":"("}),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("browser_network")
                    .with_arguments(arguments(bad.clone())),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{bad}: {text}");
        assert!(text.contains("INVALID_ARGUMENT"), "{bad}: {text}");
    }
    assert_eq!(
        driver
            .calls()
            .iter()
            .filter(|c| c.tool == "evaluate")
            .count(),
        upstream_calls,
        "rejected filters must not reach upstream"
    );
}

#[tokio::test]
async fn dom_inspection_uses_fixed_function_with_exact_ref() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    // Refs come from a snapshot; e2 is the textbox (backend node 2).
    let snapshot = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snapshot.is_error.unwrap_or(false));

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_dom")
                .with_arguments(arguments(serde_json::json!({"ref":"e2"}))),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    // The opaque ref resolves to backend node 2 of the current generation.
    let details_call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "nodeDetails")
        .expect("browser_dom must resolve the ref through nodeDetails");
    assert_eq!(details_call.arguments["backendNodeId"], 2);
    // outerHTML comes from the fixed redaction script bound to the recovered
    // selector as data.
    let eval_call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "evaluate")
        .expect("browser_dom must read outerHTML through evaluate");
    let script = eval_call.arguments["script"].as_str().unwrap();
    assert!(
        script.contains("cloneNode(true)"),
        "must serialize a clone: {script}"
    );
    assert!(
        script.contains("[redacted]"),
        "must redact sensitive values: {script}"
    );
    assert!(script.contains("clone.outerHTML"), "{script}");
    assert!(
        script.contains("document.querySelector(\"#nd-2\")"),
        "must bind the recovered selector as data: {script}"
    );
    assert!(
        !script.contains("getBoundingClientRect"),
        "must not read a layout rect under Lightpanda: {script}"
    );
    // Output merges nodeDetails with the redacted outerHTML and a null rect.
    let structured = result.structured_content.as_ref().unwrap().clone();
    let text = structured["result"].as_str().unwrap();
    assert!(text.contains("\"rect\":null"), "{text}");
    assert!(text.contains("[redacted]"), "{text}");
    assert!(!text.contains("backendNodeId"), "{text}");
    assert!(
        driver
            .calls()
            .iter()
            .all(|c| c.tool != "browser_run_code" && c.tool != "browser_run_code_unsafe"),
        "DOM inspection must never use process-level code execution"
    );
    let rows = matching_audit(&core, "browser_dom");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].detail.as_ref().unwrap()["target"], "e2");
}

#[tokio::test]
async fn evaluate_rejects_empty_expression_before_any_upstream_call() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let calls_before = driver.calls().len();

    let empty = client
        .call_tool(
            CallToolRequestParams::new("browser_evaluate")
                .with_arguments(arguments(serde_json::json!({"expression":""}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&empty).unwrap();
    assert!(empty.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("INVALID_ARGUMENT"), "{text}");
    assert_eq!(
        driver.calls().len(),
        calls_before,
        "empty expression reached the driver"
    );

    assert!(
        driver
            .calls()
            .iter()
            .all(|c| c.tool != "browser_run_code" && c.tool != "browser_run_code_unsafe"),
        "evaluate must never route through process-level code execution"
    );
}

#[tokio::test]
async fn performance_reports_no_cdp_under_lightpanda_runtime() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    let result = client
        .call_tool(CallToolRequestParams::new("browser_performance"))
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let structured = result.structured_content.as_ref().unwrap().clone();
    assert_eq!(structured["cdp_supported"], false);
    assert!(structured["cdp"].is_null(), "cdp metrics must be absent");
    assert_eq!(structured["backend"], "lightpanda");
    // Timing comes from the fixed Performance API script via native evaluate,
    // never from CDP or Chromium metrics.
    assert!(driver.calls().iter().any(|c| {
        c.tool == "evaluate"
            && c.arguments["script"]
                .as_str()
                .is_some_and(|s| s.contains("performance.getEntriesByType"))
    }));
    assert!(
        driver
            .calls()
            .iter()
            .all(|c| c.tool != "browser_run_code" && c.tool != "browser_run_code_unsafe"),
        "performance must never route through process-level code execution"
    );
}

#[tokio::test]
async fn trace_lifecycle_returns_opaque_artifacts_without_paths() {
    let (port, dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    let status = client
        .call_tool(
            CallToolRequestParams::new("browser_trace")
                .with_arguments(arguments(serde_json::json!({"action":"status"}))),
        )
        .await
        .unwrap();
    assert_eq!(
        status.structured_content.as_ref().unwrap()["tracing"],
        false
    );

    let start = client
        .call_tool(
            CallToolRequestParams::new("browser_trace")
                .with_arguments(arguments(serde_json::json!({"action":"start"}))),
        )
        .await
        .unwrap();
    assert!(!start.is_error.unwrap_or(false));
    // Tracing is app-owned: no upstream tracing primitives exist or are called.
    assert!(
        driver
            .calls()
            .iter()
            .all(|c| c.tool != "browser_start_tracing" && c.tool != "browser_stop_tracing"),
        "tracing must not call removed Playwright tracing tools"
    );

    let double = client
        .call_tool(
            CallToolRequestParams::new("browser_trace")
                .with_arguments(arguments(serde_json::json!({"action":"start"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&double).unwrap();
    assert!(double.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("INVALID_ARGUMENT"), "{text}");

    // While tracing, each routed native tool records exactly one minimal event.
    let snapshot = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snapshot.is_error.unwrap_or(false));

    let stop = client
        .call_tool(
            CallToolRequestParams::new("browser_trace")
                .with_arguments(arguments(serde_json::json!({"action":"stop"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&stop).unwrap();
    assert!(!stop.is_error.unwrap_or(false), "{text}");
    let structured = stop.structured_content.as_ref().unwrap().clone();
    assert_eq!(structured["tracing"], false);
    // Brief shape: {artifact_id, files, bytes}; no per-file array, names, or hashes.
    let artifact_id = structured["artifact_id"].as_str().unwrap();
    assert!(artifact_id.starts_with("a-"), "{text}");
    assert_eq!(artifact_id.len(), 18, "{text}");
    assert_eq!(structured["files"], 2, "{text}");
    assert!(structured["bytes"].as_u64().unwrap() > 0, "{text}");
    assert!(structured.get("artifacts").is_none(), "{text}");
    assert!(
        !text.contains("events.jsonl"),
        "trace response leaks file names"
    );
    assert!(
        !text.contains("manifest.json"),
        "trace response leaks file names"
    );
    let artifact_dir = active_artifact_dir(&dir);
    let dir_string = artifact_dir.to_string_lossy().to_string();
    assert!(
        !text.contains(&dir_string),
        "trace response leaks absolute paths"
    );
    assert!(
        !text.contains(&dir_string.replace('\\', "/")),
        "trace response leaks absolute paths"
    );

    // Artifacts on disk: one event line per routed tool call, and a manifest.
    // Events carry ts/tool/page_id only — never arguments or page content.
    let traces_dir = artifact_dir.join("traces");
    let events = std::fs::read_to_string(traces_dir.join("events.jsonl")).unwrap();
    let lines: Vec<&str> = events.lines().collect();
    assert_eq!(lines.len(), 1, "{events}");
    for line in &lines {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        let keys: std::collections::BTreeSet<String> =
            parsed.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            ["page_id", "tool", "ts"]
                .into_iter()
                .map(str::to_string)
                .collect::<std::collections::BTreeSet<String>>(),
            "trace events must stay minimal: {line}"
        );
    }
    assert!(lines[0].contains("\"tool\":\"tree\""), "{}", lines[0]);
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(traces_dir.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["schema"], "lc-trace-v1");
    assert_eq!(manifest["event_count"], lines.len() as u64);

    let stopped = client
        .call_tool(
            CallToolRequestParams::new("browser_trace")
                .with_arguments(arguments(serde_json::json!({"action":"stop"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&stopped).unwrap();
    assert!(stopped.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("INVALID_ARGUMENT"), "{text}");
}

#[tokio::test]
async fn devtools_is_unsupported_under_fixed_background_backend() {
    // Visible Debug is no longer a user-selectable run mode (v6 settings), so
    // the fixed background backend can never report DevTools support. The tool
    // must fail closed and never launch a second browser.
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let result = client
        .call_tool(CallToolRequestParams::new("browser_devtools"))
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_FEATURE_UNSUPPORTED"), "{text}");
    assert_eq!(
        driver.starts.load(Ordering::SeqCst),
        1,
        "devtools must never launch a second browser"
    );
}

#[tokio::test]
async fn unsupported_upload_fails_closed_without_staging() {
    let (port, dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let workspace_root = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace_root).unwrap();
    std::fs::write(workspace_root.join("upload.txt"), b"upload-body").unwrap();
    let ws = lc_core::workspace_repo::insert_workspace(
        &core.db,
        "Web",
        &workspace_root.to_string_lossy(),
    )
    .unwrap();
    lc_core::workspace_repo::update_capabilities(&core.db, &ws.id, true, true).unwrap();

    let client = client(port).await;
    open_browser(&client).await;

    // Even a fully authorized workspace source fails closed: the Lightpanda
    // runtime has no upload primitive.
    let result = client
        .call_tool(CallToolRequestParams::new("browser_upload").with_arguments(arguments(
            serde_json::json!({"workspace_id":ws.id,"sources":[{"kind":"workspace","path":"upload.txt"}]}),
        )))
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_FEATURE_UNSUPPORTED"), "{text}");

    // No staging side effect may happen before the unsupported return.
    let artifact_dir = active_artifact_dir(&dir);
    let staging = artifact_dir.join("upload-staging");
    assert!(
        !staging.exists(),
        "unsupported upload created a staging directory"
    );
    assert!(
        driver.calls().is_empty(),
        "unsupported upload reached the driver: {:?}",
        driver.calls()
    );
    let rows = matching_audit(&core, "browser_upload");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, "error");
}

#[tokio::test]
async fn unsupported_downloads_and_save_fail_closed_without_touching_workspace() {
    let (port, dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let workspace_root = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let ws = lc_core::workspace_repo::insert_workspace(
        &core.db,
        "Web",
        &workspace_root.to_string_lossy(),
    )
    .unwrap();
    lc_core::workspace_repo::update_capabilities(&core.db, &ws.id, true, true).unwrap();

    let client = client(port).await;
    open_browser(&client).await;
    // A file that a Playwright runtime would have reported as a download.
    std::fs::write(
        active_artifact_dir(&dir).join("result.txt"),
        b"download-body",
    )
    .unwrap();

    let downloads = client
        .call_tool(CallToolRequestParams::new("browser_downloads"))
        .await
        .unwrap();
    let text = serde_json::to_string(&downloads).unwrap();
    assert!(downloads.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_FEATURE_UNSUPPORTED"), "{text}");

    let save = client.call_tool(CallToolRequestParams::new("browser_save_download_to_workspace").with_arguments(arguments(serde_json::json!({"workspace_id":ws.id,"download_id":"d-0000000000000000-1","path":"saved/result.txt"})))).await.unwrap();
    let text = serde_json::to_string(&save).unwrap();
    assert!(save.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_FEATURE_UNSUPPORTED"), "{text}");

    // The destination must stay untouched and no driver call may happen.
    assert!(
        !workspace_root.join("saved/result.txt").exists(),
        "unsupported save wrote into the workspace"
    );
    assert!(
        driver.calls().is_empty(),
        "unsupported downloads reached the driver: {:?}",
        driver.calls()
    );
    assert_eq!(matching_audit(&core, "browser_downloads").len(), 1);
    assert_eq!(
        matching_audit(&core, "browser_save_download_to_workspace").len(),
        1
    );
}

#[tokio::test]
async fn run_steps_rejects_over_limit_before_driver() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let before = driver.calls().len();

    let steps: Vec<serde_json::Value> = (0..21)
        .map(|_| serde_json::json!({"type":"snapshot"}))
        .collect();
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_run_steps")
                .with_arguments(arguments(serde_json::json!({"steps": steps}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("INVALID_ARGUMENT"), "{text}");
    assert_eq!(
        driver.calls().len(),
        before,
        "over-limit batch reached the driver"
    );
    let rows = matching_audit(&core, "browser_run_steps");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, "error");
}

#[tokio::test]
async fn run_steps_stops_on_first_failure_and_audits_failure() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    // fill invalidates snapshot refs, so the following click on a pre-fill ref
    // must fail closed with BROWSER_STALE_REF and stop the batch.
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_run_steps").with_arguments(arguments(
                serde_json::json!({"steps":[
                    {"type":"navigate","url":"https://example.test/flow"},
                    {"type":"snapshot"},
                    {"type":"fill","ref_id":"e2","text":"batch-secret"},
                    {"type":"click","ref_id":"e1"},
                    {"type":"snapshot"}
                ]}),
            )),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(!result.is_error.unwrap_or(false), "{text}");
    let structured = result.structured_content.as_ref().unwrap().clone();
    assert_eq!(structured["ok"], false);
    assert_eq!(structured["failed_index"], 3);
    assert_eq!(structured["status"], "failed");
    assert_eq!(structured["error"]["code"], "BROWSER_STALE_REF");
    assert_eq!(structured["results"].as_array().unwrap().len(), 3);
    assert_eq!(
        driver.calls().iter().filter(|c| c.tool == "tree").count(),
        2,
        "steps after the failure were executed"
    );

    let rows = matching_audit(&core, "browser_run_steps");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].decision, "error",
        "failed batch must not audit as allow"
    );
    assert_eq!(rows[0].error_code.as_deref(), Some("BROWSER_STALE_REF"));
    let detail = rows[0].detail.as_ref().unwrap();
    assert_eq!(detail["step_count"], 5);
    assert_eq!(detail["failed_index"], 3);
    assert_eq!(detail["status"], "failed");
    assert_eq!(
        detail["step_types"],
        serde_json::json!(["navigate", "snapshot", "fill", "click", "snapshot"])
    );
    assert!(
        !serde_json::to_string(detail)
            .unwrap()
            .contains("batch-secret"),
        "run_steps audit leaks filled text"
    );
}

#[tokio::test]
async fn run_steps_process_loss_is_not_replayed() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    driver.fail(
        "click",
        CoreError::new(ErrorCode::BrowserProcessLost, "browser process was lost"),
    );

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_run_steps").with_arguments(arguments(
                serde_json::json!({"steps":[{"type":"snapshot"},{"type":"click","ref_id":"e1"}]}),
            )),
        )
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap().clone();
    assert_eq!(structured["ok"], false);
    assert_eq!(structured["failed_index"], 1);
    assert_eq!(structured["error"]["code"], "BROWSER_PROCESS_LOST");
    assert_eq!(
        driver.calls().iter().filter(|c| c.tool == "click").count(),
        1,
        "failed step was replayed"
    );
    let rows = matching_audit(&core, "browser_run_steps");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, "error");
}

#[tokio::test]
async fn run_steps_rejects_disallowed_step_types() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    for bad in [
        serde_json::json!({"type":"evaluate","expression":"() => 1"}),
        serde_json::json!({"type":"upload","sources":[]}),
        serde_json::json!({"type":"dialog","accept":true}),
        serde_json::json!({"type":"trace","action":"start"}),
        serde_json::json!({"type":"devtools"}),
        serde_json::json!({"type":"save_download","download_id":"d-1","path":"x.txt"}),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("browser_run_steps")
                    .with_arguments(arguments(serde_json::json!({"steps":[bad.clone()]}))),
            )
            .await;
        match result {
            Err(_) => {}
            Ok(result) => {
                assert!(
                    result.is_error.unwrap_or(false),
                    "disallowed step type accepted: {bad}"
                );
            }
        }
    }
    assert!(
        driver.calls().is_empty(),
        "disallowed step reached the driver: {:?}",
        driver.calls()
    );
    // Protocol-level rejections happen before the handler: no audit rows.
    assert!(matching_audit(&core, "browser_run_steps").is_empty());
}

#[tokio::test]
async fn run_steps_preflight_validates_every_step_before_step_zero() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    // A side-effecting step 0 plus one invalid later step must reject the
    // whole batch before step 0 runs.
    for invalid_step in [
        serde_json::json!({"type":"navigate","url":"not a url"}),
        serde_json::json!({"type":"click","ref_id":"not-a-ref"}),
        serde_json::json!({"type":"fill","ref_id":"e1","text":""}),
        serde_json::json!({"type":"press","key":"  "}),
        serde_json::json!({"type":"select","ref_id":"e1","values":[]}),
        serde_json::json!({"type":"wait","condition":{"kind":"delay","milliseconds":0},"timeout_ms":1000}),
        serde_json::json!({"type":"wait","condition":{"kind":"delay","milliseconds":5000},"timeout_ms":1000}),
        serde_json::json!({"type":"set_viewport","width":320,"height":568}),
        serde_json::json!({"type":"screenshot","mode":"viewport"}),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("browser_run_steps").with_arguments(arguments(
                    serde_json::json!({"steps":[
                        {"type":"navigate","url":"https://example.test/first"},
                        invalid_step.clone()
                    ]}),
                )),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{invalid_step}: {text}");
        assert!(
            text.contains("INVALID_ARGUMENT") || text.contains("BROWSER_FEATURE_UNSUPPORTED"),
            "{invalid_step}: {text}"
        );
    }
    assert!(
        driver.calls().is_empty(),
        "preflight-rejected batch ran step 0: {:?}",
        driver.calls()
    );
    let rows = matching_audit(&core, "browser_run_steps");
    assert_eq!(rows.len(), 9);
    for row in &rows {
        assert_eq!(row.decision, "error");
    }
}

// ---------------------------------------------------------------------------
// Native-first wrappers (Task 10): every tool maps to a pinned Lightpanda
// native primitive, backend node ids are rewritten to opaque refs, content is
// bounded, and audit rows carry byte counts / opaque targets only.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn native_first_markdown_maps_to_native_with_ref_scope_and_bounded_bytes() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    // Whole-page markdown maps to the native `markdown` tool.
    driver.respond("markdown", "# Title\n\nBody text".to_string());
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_markdown")
                .with_arguments(arguments(serde_json::json!({}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(!result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("# Title"), "{text}");
    assert!(text.contains("\"truncated\":false"), "{text}");
    assert!(driver
        .calls()
        .iter()
        .any(|c| c.tool == "markdown" && c.arguments.get("selector").is_none()));

    // A snapshot ref scopes the read through a recovered selector.
    let snapshot = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snapshot.is_error.unwrap_or(false));
    driver.respond("markdown", "scoped body".to_string());
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_markdown")
                .with_arguments(arguments(serde_json::json!({"ref":"e2"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(!result.is_error.unwrap_or(false), "{text}");
    assert!(driver
        .calls()
        .iter()
        .any(|c| c.tool == "nodeDetails" && c.arguments["backendNodeId"] == 2));
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "markdown")
        .unwrap();
    assert_eq!(call.arguments["selector"], "#nd-2");

    // Caller-supplied caps truncate and are bounded to the hard maximum.
    driver.respond("markdown", "0123456789".to_string());
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_markdown")
                .with_arguments(arguments(serde_json::json!({"max_bytes":5}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(text.contains("01234"), "{text}");
    assert!(text.contains("\"truncated\":true"), "{text}");

    let calls_before = driver.calls().len();
    for bad in [
        serde_json::json!({"max_bytes":0}),
        serde_json::json!({"max_bytes":262145}),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("browser_markdown").with_arguments(arguments(bad)),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{text}");
        assert!(text.contains("INVALID_ARGUMENT"), "{text}");
    }
    assert_eq!(
        driver.calls().len(),
        calls_before,
        "invalid cap reached the driver"
    );

    // Audit stores request metadata only, never page content.
    let rows = matching_audit(&core, "browser_markdown");
    assert_eq!(rows.len(), 5);
    for row in &rows {
        let detail = serde_json::to_string(row.detail.as_ref().unwrap()).unwrap();
        assert!(
            !detail.contains("Title")
                && !detail.contains("scoped body")
                && !detail.contains("01234"),
            "markdown audit must not carry content: {detail}"
        );
    }
}

#[tokio::test]
async fn native_first_html_maps_to_native_with_ref_scope_and_bounded_bytes() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    driver.respond("html", "<html><body><p>hi</p></body></html>".to_string());
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_html")
                .with_arguments(arguments(serde_json::json!({}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(!result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("<p>hi</p>"), "{text}");
    assert!(driver
        .calls()
        .iter()
        .any(|c| c.tool == "html" && c.arguments.get("selector").is_none()));

    let snapshot = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snapshot.is_error.unwrap_or(false));
    driver.respond("html", "<input>".to_string());
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_html")
                .with_arguments(arguments(serde_json::json!({"ref":"e3"}))),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    assert!(driver
        .calls()
        .iter()
        .any(|c| c.tool == "nodeDetails" && c.arguments["backendNodeId"] == 3));
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "html")
        .unwrap();
    assert_eq!(call.arguments["selector"], "#nd-3");

    let calls_before = driver.calls().len();
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_html")
                .with_arguments(arguments(serde_json::json!({"max_bytes":0}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("INVALID_ARGUMENT"), "{text}");
    assert_eq!(
        driver.calls().len(),
        calls_before,
        "invalid cap reached the driver"
    );

    let rows = matching_audit(&core, "browser_html");
    assert_eq!(rows.len(), 3);
    for row in &rows {
        let detail = serde_json::to_string(row.detail.as_ref().unwrap()).unwrap();
        assert!(!detail.contains("<p>hi</p>"), "{detail}");
    }
}

#[tokio::test]
async fn native_first_links_normalizes_backend_ids_to_actionable_refs() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    driver.respond(
        "links",
        "[{\"backendNodeId\":7,\"text\":\"Docs\",\"href\":\"https://example.com/docs\"},{\"backendNodeId\":8,\"text\":\"Blog\",\"href\":\"https://example.com/blog\"}]"
            .to_string(),
    );
    let result = client
        .call_tool(CallToolRequestParams::new("browser_links"))
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = truncated_text(&result);
    assert!(driver.calls().iter().any(|c| c.tool == "links"));
    assert!(text.contains("\"ref\":\"e1\""), "{text}");
    assert!(text.contains("\"ref\":\"e2\""), "{text}");
    assert!(!text.contains("backendNodeId"), "{text}");

    // Normalized refs resolve back to the same backend nodes.
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_click")
                .with_arguments(arguments(serde_json::json!({"ref":"e1"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(!result.is_error.unwrap_or(false), "{text}");
    assert!(driver
        .calls()
        .iter()
        .any(|c| c.tool == "nodeDetails" && c.arguments["backendNodeId"] == 7));
    assert!(driver
        .calls()
        .iter()
        .any(|c| c.tool == "click" && c.arguments["selector"] == "#nd-7"));

    let rows = matching_audit(&core, "browser_links");
    assert_eq!(rows.len(), 1);
    if let Some(detail) = &rows[0].detail {
        let detail = serde_json::to_string(detail).unwrap();
        assert!(
            !detail.contains("Docs") && !detail.contains("example.com"),
            "{detail}"
        );
    }
}

#[tokio::test]
async fn native_first_find_element_validates_criteria_and_normalizes_refs() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    let calls_before = driver.calls().len();
    for bad in [
        serde_json::json!({}),
        serde_json::json!({"role":"  "}),
        serde_json::json!({"name":""}),
        serde_json::json!({"role":"","name":" "}),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("browser_find_element").with_arguments(arguments(bad)),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{text}");
        assert!(text.contains("INVALID_ARGUMENT"), "{text}");
    }
    assert_eq!(
        driver.calls().len(),
        calls_before,
        "criterion-less find reached the driver"
    );

    driver.respond(
        "findElement",
        "[{\"backendNodeId\":9,\"tagName\":\"button\",\"role\":\"button\",\"name\":\"Go\"}]"
            .to_string(),
    );
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_find_element").with_arguments(arguments(
                serde_json::json!({"role":" button ","name":" Go "}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = truncated_text(&result);
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "findElement")
        .unwrap();
    assert_eq!(call.arguments["role"], "button");
    assert_eq!(call.arguments["name"], "Go");
    assert!(text.contains("\"ref\":\"e1\""), "{text}");
    assert!(!text.contains("backendNodeId"), "{text}");

    let rows = matching_audit(&core, "browser_find_element");
    let ok_rows: Vec<_> = rows.iter().filter(|r| r.decision == "allow").collect();
    assert_eq!(ok_rows.len(), 1);
    let detail = ok_rows[0].detail.as_ref().unwrap();
    assert_eq!(detail["role_bytes"], 8);
    assert_eq!(detail["name_bytes"], 4);
    assert!(
        !serde_json::to_string(detail).unwrap().contains("Go"),
        "find_element audit must be byte counts only"
    );
}

#[tokio::test]
async fn native_first_interactive_elements_normalizes_backend_ids() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    driver.respond(
        "interactiveElements",
        "[{\"backendNodeId\":5,\"tagName\":\"input\",\"role\":\"textbox\",\"name\":\"Name\",\"type\":\"native\"},{\"backendNodeId\":6,\"tagName\":\"button\",\"role\":\"button\",\"name\":\"Submit\",\"type\":\"native\"}]"
            .to_string(),
    );
    let result = client
        .call_tool(CallToolRequestParams::new("browser_interactive_elements"))
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = truncated_text(&result);
    assert!(driver
        .calls()
        .iter()
        .any(|c| c.tool == "interactiveElements"));
    assert!(text.contains("\"ref\":\"e1\""), "{text}");
    assert!(text.contains("\"ref\":\"e2\""), "{text}");
    assert!(!text.contains("backendNodeId"), "{text}");
    assert!(text.contains("Submit"), "{text}");
}

#[tokio::test]
async fn native_first_extract_validates_schema_and_parses_structured_result() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    // Non-object roots and invalid shapes are rejected before any native call.
    let calls_before = driver.calls().len();
    for bad in [
        serde_json::json!({"schema":"title"}),
        serde_json::json!({"schema":["h1"]}),
        serde_json::json!({"schema":42}),
    ] {
        let result = client
            .call_tool(CallToolRequestParams::new("browser_extract").with_arguments(arguments(bad)))
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{text}");
        assert!(text.contains("INVALID_ARGUMENT"), "{text}");
    }
    assert_eq!(
        driver.calls().len(),
        calls_before,
        "invalid schema reached the driver"
    );

    // A valid schema is serialized once into the upstream schema string and
    // the structured result is parsed JSON, not a raw string.
    let schema = serde_json::json!({
        "title": {"selector": "h1"},
        "prices": {"selector": ".price", "limit": 5, "fields": {"amount": {"selector": ".amt"}}}
    });
    driver.respond(
        "extract",
        "{\"title\":\"Hello\",\"prices\":[{\"amount\":\"$10\"}]}".to_string(),
    );
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_extract")
                .with_arguments(arguments(serde_json::json!({"schema": schema}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(!result.is_error.unwrap_or(false), "{text}");
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "extract")
        .unwrap();
    let encoded = call.arguments["schema"]
        .as_str()
        .expect("upstream schema must be a string");
    let parsed: serde_json::Value = serde_json::from_str(encoded).unwrap();
    assert_eq!(parsed, schema);
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["data"]["title"], "Hello");
    assert_eq!(structured["data"]["prices"][0]["amount"], "$10");

    // Non-JSON upstream text is wrapped as a string value, never dropped.
    driver.respond("extract", "not json".to_string());
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_extract").with_arguments(arguments(
                serde_json::json!({"schema": {"title": {"selector": "h1"}}}),
            )),
        )
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["data"], "not json");

    // Audit records schema bytes only.
    let rows = matching_audit(&core, "browser_extract");
    let ok_rows: Vec<_> = rows.iter().filter(|r| r.decision == "allow").collect();
    assert_eq!(ok_rows.len(), 2);
    for row in &ok_rows {
        let detail = serde_json::to_string(row.detail.as_ref().unwrap()).unwrap();
        assert!(detail.contains("schema_bytes"), "{detail}");
        assert!(
            !detail.contains("h1") && !detail.contains("selector"),
            "{detail}"
        );
    }
}

#[tokio::test]
async fn native_first_structured_data_maps_to_native_tool() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    driver.respond(
        "structuredData",
        "{\"jsonLd\":[{\"@type\":\"Article\"}],\"openGraph\":{\"title\":\"OG\"}}".to_string(),
    );
    let result = client
        .call_tool(CallToolRequestParams::new("browser_structured_data"))
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(!result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("Article"), "{text}");
    assert!(text.contains("\"truncated\":false"), "{text}");
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "structuredData")
        .unwrap();
    assert!(call.arguments.as_object().is_some_and(|m| m.is_empty()));
}

#[tokio::test]
async fn native_first_detect_forms_redacts_passwords_and_normalizes_refs() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    let forms = serde_json::json!([{
        "backendNodeId": 2,
        "action": "/login",
        "method": "post",
        "fields": [
            {"backendNodeId": 3, "tagName": "input", "name": "user", "inputType": "text",
             "required": true, "disabled": false, "value": "visible-user"},
            {"backendNodeId": 4, "tagName": "input", "name": "pass", "inputType": "password",
             "required": true, "disabled": false, "value": "hunter2-secret"}
        ]
    }]);
    driver.respond("detectForms", serde_json::to_string(&forms).unwrap());
    let result = client
        .call_tool(CallToolRequestParams::new("browser_detect_forms"))
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = truncated_text(&result);
    assert!(driver.calls().iter().any(|c| c.tool == "detectForms"));
    assert!(
        !text.contains("hunter2-secret"),
        "password value leaked: {text}"
    );
    assert!(text.contains("*****"), "{text}");
    assert!(
        text.contains("visible-user"),
        "non-password values stay: {text}"
    );
    assert!(!text.contains("backendNodeId"), "{text}");
    assert!(text.contains("\"ref\":\"e1\""), "{text}");
    assert!(text.contains("\"ref\":\"e2\""), "{text}");
    assert!(text.contains("\"ref\":\"e3\""), "{text}");

    let rows = matching_audit(&core, "browser_detect_forms");
    assert_eq!(rows.len(), 1);
    if let Some(detail) = &rows[0].detail {
        let detail = serde_json::to_string(detail).unwrap();
        assert!(
            !detail.contains("hunter2") && !detail.contains("visible-user"),
            "{detail}"
        );
    }
}

#[tokio::test]
async fn native_first_scroll_maps_offsets_and_ref_without_invalidation() {
    let (port, _dir, _core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let snapshot = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snapshot.is_error.unwrap_or(false));

    // Page scroll carries offsets only.
    driver.respond("scroll", "scrolled".to_string());
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_scroll")
                .with_arguments(arguments(serde_json::json!({"y":500}))),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "scroll")
        .unwrap();
    assert_eq!(call.arguments["y"], 500);
    assert!(call.arguments.get("backendNodeId").is_none());
    assert!(call.arguments.get("selector").is_none());

    // Ref scroll targets the backend node directly (upstream scroll has no
    // selector support); the id never leaves the manager.
    driver.respond("scroll", "scrolled".to_string());
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_scroll")
                .with_arguments(arguments(serde_json::json!({"ref":"e2","x":10,"y":20}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(!result.is_error.unwrap_or(false), "{text}");
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "scroll")
        .unwrap();
    assert_eq!(call.arguments["backendNodeId"], 2);
    assert_eq!(call.arguments["x"], 10);
    assert_eq!(call.arguments["y"], 20);
    assert!(
        driver.calls().iter().all(|c| c.tool != "nodeDetails"),
        "scroll must not need selector recovery"
    );

    // Out-of-bounds offsets are rejected before any native call.
    let calls_before = driver.calls().len();
    for bad in [
        serde_json::json!({"y":10_000_001}),
        serde_json::json!({"x":-10_000_001}),
    ] {
        let result = client
            .call_tool(CallToolRequestParams::new("browser_scroll").with_arguments(arguments(bad)))
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{text}");
        assert!(text.contains("INVALID_ARGUMENT"), "{text}");
    }
    assert_eq!(
        driver.calls().len(),
        calls_before,
        "out-of-bounds scroll reached the driver"
    );

    // Scrolling never mutates the DOM: existing refs stay valid.
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_click")
                .with_arguments(arguments(serde_json::json!({"ref":"e4"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(!result.is_error.unwrap_or(false), "{text}");
    assert!(driver
        .calls()
        .iter()
        .any(|c| c.tool == "click" && c.arguments["selector"] == "#nd-4"));
}

#[tokio::test]
async fn native_first_set_checked_recovers_selector_and_invalidates_refs() {
    let (port, _dir, core, driver, _browser) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let snapshot = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snapshot.is_error.unwrap_or(false));

    // `checked` defaults to true; the selector is recovered via nodeDetails.
    driver.respond("setChecked", "ok".to_string());
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_set_checked")
                .with_arguments(arguments(serde_json::json!({"ref":"e3"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(!result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("updated the checked state"), "{text}");
    assert!(driver
        .calls()
        .iter()
        .any(|c| c.tool == "nodeDetails" && c.arguments["backendNodeId"] == 3));
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "setChecked")
        .unwrap();
    assert_eq!(call.arguments["selector"], "#nd-3");
    assert_eq!(call.arguments["checked"], true);

    // Side-effecting: refs are invalidated afterwards.
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_click")
                .with_arguments(arguments(serde_json::json!({"ref":"e3"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_STALE_REF"), "{text}");

    // Re-snapshot and pass checked=false explicitly.
    let snapshot = client
        .call_tool(CallToolRequestParams::new("browser_snapshot"))
        .await
        .unwrap();
    assert!(!snapshot.is_error.unwrap_or(false));
    driver.respond("setChecked", "ok".to_string());
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_set_checked")
                .with_arguments(arguments(serde_json::json!({"ref":"e3","checked":false}))),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "setChecked")
        .unwrap();
    assert_eq!(call.arguments["checked"], false);

    // Audit records the opaque target and requested state only.
    // list_recent is newest-first: the checked=false call is row 0.
    let rows = matching_audit(&core, "browser_set_checked");
    let ok_rows: Vec<_> = rows.iter().filter(|r| r.decision == "allow").collect();
    assert_eq!(ok_rows.len(), 2);
    assert_eq!(ok_rows[0].detail.as_ref().unwrap()["checked"], false);
    assert_eq!(ok_rows[1].detail.as_ref().unwrap()["target"], "e3");
    assert_eq!(ok_rows[1].detail.as_ref().unwrap()["checked"], true);
    for row in &ok_rows {
        let detail = serde_json::to_string(row.detail.as_ref().unwrap()).unwrap();
        assert!(!detail.contains("#nd-3"), "{detail}");
    }
}
