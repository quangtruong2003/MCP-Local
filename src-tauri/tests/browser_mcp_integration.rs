use std::{
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
        driver::{BrowserDriver, BrowserDriverStatus},
        manager::BrowserManager,
        types::BrowserLaunchSpec,
    },
    screen_capture::{CaptureTarget, CapturedImage, ScreenCaptureProvider},
    settings::{
        AgentPolicyState, AgentSettingsController, AppSettings, BrowserEngine, BrowserRunMode,
    },
};
use rmcp::{model::CallToolRequestParams, transport::StreamableHttpClientTransport, ServiceExt};

#[derive(Debug, Clone)]
struct RecordedCall {
    tool: String,
    arguments: serde_json::Value,
}

#[derive(Debug)]
struct FakeTab {
    title: String,
    url: String,
    active: bool,
}

#[derive(Debug, Default)]
struct FakeBrowserDriver {
    calls: std::sync::Mutex<Vec<RecordedCall>>,
    tabs: std::sync::Mutex<Vec<FakeTab>>,
    artifact_dir: std::sync::Mutex<Option<PathBuf>>,
    launches: std::sync::Mutex<Vec<BrowserLaunchSpec>>,
    failures: std::sync::Mutex<Vec<(String, lc_core::error::CoreError)>>,
    responses: std::sync::Mutex<Vec<(String, String)>>,
    screenshot_block: std::sync::Mutex<Option<rmcp::model::ContentBlock>>,
    starts: AtomicUsize,
    stops: AtomicUsize,
    cdp_calls: AtomicUsize,
}

impl FakeBrowserDriver {
    fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }

    fn artifact_dir(&self) -> PathBuf {
        self.artifact_dir
            .lock()
            .unwrap()
            .clone()
            .expect("browser must be started")
    }

    fn launches(&self) -> Vec<BrowserLaunchSpec> {
        self.launches.lock().unwrap().clone()
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

    fn tabs_text(&self) -> String {
        self.tabs
            .lock()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let current = if tab.active { " (current)" } else { "" };
                format!("- {index}:{current} [{}]({})", tab.title, tab.url)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[async_trait::async_trait]
impl BrowserDriver for FakeBrowserDriver {
    async fn start(
        &self,
        spec: BrowserLaunchSpec,
    ) -> Result<BrowserDriverStatus, lc_core::error::CoreError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        *self.artifact_dir.lock().unwrap() = Some(spec.artifact_dir.clone());
        self.launches.lock().unwrap().push(spec.clone());
        let mut tabs = self.tabs.lock().unwrap();
        if tabs.is_empty() {
            tabs.push(FakeTab {
                title: "Blank".into(),
                url: "about:blank".into(),
                active: true,
            });
        }
        Ok(BrowserDriverStatus {
            running: true,
            actual_engine: spec.engine,
            sidecar_version: Some("test".into()),
            browser_version: Some("test-browser".into()),
            child_pid: None,
        })
    }

    async fn stop(&self) -> Result<(), lc_core::error::CoreError> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn call(
        &self,
        tool: &'static str,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> Result<rmcp::model::CallToolResult, lc_core::error::CoreError> {
        self.calls.lock().unwrap().push(RecordedCall {
            tool: tool.to_string(),
            arguments: serde_json::Value::Object(arguments.clone()),
        });

        let failure_pos = {
            self.failures
                .lock()
                .unwrap()
                .iter()
                .position(|(name, _)| name == tool)
        };
        if let Some(position) = failure_pos {
            let (_, error) = self.failures.lock().unwrap().remove(position);
            return Err(error);
        }
        let response_pos = {
            self.responses
                .lock()
                .unwrap()
                .iter()
                .position(|(name, _)| name == tool)
        };
        if let Some(position) = response_pos {
            let (_, text) = self.responses.lock().unwrap().remove(position);
            return Ok(rmcp::model::CallToolResult::success(vec![
                rmcp::model::ContentBlock::text(text),
            ]));
        }

        if tool == "browser_navigate" {
            if let Some(url) = arguments.get("url").and_then(|value| value.as_str()) {
                let mut tabs = self.tabs.lock().unwrap();
                if let Some(active) = tabs.iter_mut().find(|tab| tab.active) {
                    active.url = url.to_string();
                    active.title = "Navigated".into();
                }
            }
        }

        if tool == "browser_tabs" {
            let action = arguments
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or("list");
            match action {
                "new" => {
                    let mut tabs = self.tabs.lock().unwrap();
                    for tab in tabs.iter_mut() {
                        tab.active = false;
                    }
                    tabs.push(FakeTab {
                        title: "New".into(),
                        url: arguments
                            .get("url")
                            .and_then(|value| value.as_str())
                            .unwrap_or("about:blank")
                            .to_string(),
                        active: true,
                    });
                }
                "select" => {
                    let index = arguments
                        .get("index")
                        .and_then(|value| value.as_u64())
                        .unwrap() as usize;
                    let mut tabs = self.tabs.lock().unwrap();
                    for (i, tab) in tabs.iter_mut().enumerate() {
                        tab.active = i == index;
                    }
                }
                "close" => {
                    let index = arguments
                        .get("index")
                        .and_then(|value| value.as_u64())
                        .unwrap() as usize;
                    let mut tabs = self.tabs.lock().unwrap();
                    if index < tabs.len() {
                        let was_active = tabs[index].active;
                        tabs.remove(index);
                        if was_active && !tabs.is_empty() {
                            tabs[0].active = true;
                        }
                    }
                }
                _ => {}
            }
            return Ok(rmcp::model::CallToolResult::success(vec![
                rmcp::model::ContentBlock::text(self.tabs_text()),
            ]));
        }

        match tool {
            "browser_snapshot" => return Ok(rmcp::model::CallToolResult::success(vec![rmcp::model::ContentBlock::text("- button 'Save' [target=e1]\n- textbox 'Name' [target=e2]")])),
            "browser_take_screenshot" => {
                let block = self.screenshot_block.lock().unwrap().take();
                return Ok(rmcp::model::CallToolResult::success(vec![
                    block.unwrap_or_else(Self::png_block),
                ]));
            }
            // Mirrors pinned @playwright/mcp 0.0.79 output: a `Total messages:`
            // header, a blank separator line, then `[LEVEL] text @ url:line` rows.
            "browser_console_messages" => return Ok(rmcp::model::CallToolResult::success(vec![rmcp::model::ContentBlock::text("Total messages: 3 (Errors: 1, Warnings: 1)\n\n[INFO] hello @ https://example.test/app.js:1\n[WARNING] warn @ https://example.test/app.js:2\n[ERROR] boom @ https://example.test/app.js:3")])),
            // Mirrors pinned @playwright/mcp 0.0.79 output: numbered
            // `N. [METHOD] URL => [STATUS] STATUS_TEXT` rows. Real 0.0.79 list
            // output carries no header lines; the sensitive ones below are kept
            // deliberately so redaction stays covered defense-in-depth.
            "browser_network_requests" => return Ok(rmcp::model::CallToolResult::success(vec![rmcp::model::ContentBlock::text("1. [GET] https://example.test/api => [200] OK\nAuthorization: Bearer secret\nCookie: session=secret")])),
            "browser_evaluate" => {
                let f = arguments.get("function").and_then(|v| v.as_str()).unwrap_or_default();
                let text = if f.contains("document.readyState") { "complete" }
                    else if f.contains("location.href") { "https://example.test/path" }
                    else if f.contains("getBoundingClientRect") && f.contains("visibility") { "true" }
                    else if f.contains("performance.getEntriesByType") { "{\"navigation\":[],\"resources\":[]}" }
                    else if f.contains("outerHTML") { "{\"tagName\":\"INPUT\",\"attrs\":{\"value\":\"[redacted]\"}}" }
                    else { "eval-ok" };
                return Ok(rmcp::model::CallToolResult::success(vec![rmcp::model::ContentBlock::text(text)]));
            }
            "browser_stop_tracing" => {
                let dir = self.artifact_dir();
                std::fs::create_dir_all(dir.join("traces")).unwrap();
                std::fs::write(dir.join("traces").join("trace.zip"), b"trace").unwrap();
            }
            _ => {}
        }

        Ok(rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::text("ok"),
        ]))
    }

    async fn chromium_performance_metrics(
        &self,
    ) -> Result<serde_json::Value, lc_core::error::CoreError> {
        self.cdp_calls.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({"metrics": []}))
    }

    fn launch_devtools_auto_open(&self, spec: &BrowserLaunchSpec) -> bool {
        // Mirrors the real Playwright driver: headed Chromium-family launches
        // carry --auto-open-devtools-for-tabs.
        spec.run_mode == BrowserRunMode::VisibleDebug
            && matches!(
                spec.engine,
                BrowserEngine::BundledChromium | BrowserEngine::Chrome | BrowserEngine::Edge
            )
    }

    async fn device_descriptor(
        &self,
        name: &str,
    ) -> Result<serde_json::Value, lc_core::error::CoreError> {
        if name == "Test Phone" {
            Ok(
                serde_json::json!({"viewport":{"width":390,"height":844},"deviceScaleFactor":3.0,"isMobile":true,"hasTouch":true,"userAgent":"TestPhone/1.0"}),
            )
        } else {
            Err(lc_core::error::CoreError::new(
                lc_core::error::ErrorCode::BrowserFeatureUnsupported,
                "preset not found",
            ))
        }
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
    Arc<FakeBrowserDriver>,
) {
    let dir = tempfile::tempdir().unwrap();
    local_workspace_mcp_manager::settings::save_app_settings(dir.path(), &settings).unwrap();
    let core = Arc::new(lc_core::CoreServices::open(&dir.path().join("test.db")).unwrap());
    let policy = Arc::new(AgentPolicyState::from_settings(&settings));
    let controller = Arc::new(AgentSettingsController::new(
        dir.path().to_path_buf(),
        policy.clone(),
    ));
    let driver = Arc::new(FakeBrowserDriver::default());
    let browser = Arc::new(
        BrowserManager::new(
            core.clone(),
            dir.path().to_path_buf(),
            &settings,
            driver.clone(),
        )
        .unwrap(),
    );
    let capture: Arc<dyn ScreenCaptureProvider> = Arc::new(EmptyCapture);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let core_for_server = core.clone();
    let service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        move || {
            Ok(local_workspace_mcp_manager::server::WorkspaceServer::new(
                core_for_server.clone(),
                policy.clone(),
                controller.clone(),
                browser.clone(),
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
    (port, dir, core, driver)
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

fn matching_audit(core: &lc_core::CoreServices, op: &str) -> Vec<lc_core::models::AuditEvent> {
    lc_core::audit::list_recent(&core.db, 100)
        .unwrap()
        .into_iter()
        .filter(|row| row.op == op)
        .collect()
}

#[tokio::test]
async fn browser_discovery_exposes_exactly_29_public_browser_tools() {
    let (port, _dir, _core, _driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let names: std::collections::BTreeSet<String> = client
        .list_all_tools()
        .await
        .unwrap()
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
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(names.len(), 29, "browser tool count changed: {names:?}");
    assert_eq!(names, expected);
}

#[tokio::test]
async fn task7_to_11_browser_tools_smoke_through_mcp_and_preserve_redaction() {
    let (port, dir, core, driver) = spawn(AppSettings::default()).await;
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
        (
            "browser_dialog",
            serde_json::json!({"accept":true,"prompt_text":"private prompt"}),
        ),
        (
            "browser_set_viewport",
            serde_json::json!({"width":375,"height":812}),
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
    }

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

    let screenshot = client
        .call_tool(
            CallToolRequestParams::new("browser_screenshot").with_arguments(arguments(
                serde_json::json!({"mode":"viewport","max_edge":2}),
            )),
        )
        .await
        .unwrap();
    assert!(
        !screenshot.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&screenshot).unwrap()
    );
    assert!(screenshot
        .content
        .iter()
        .any(|block| block.as_image().is_some()));
    assert_eq!(
        screenshot.structured_content.as_ref().unwrap()["returned_width"],
        2
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
    assert!(
        !emulate.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&emulate).unwrap()
    );
    let after_session = emulate.structured_content.as_ref().unwrap()["session_id"]
        .as_str()
        .unwrap();
    assert_ne!(before_session, after_session);
    assert!(driver.starts.load(Ordering::SeqCst) >= 2);

    let upload = client
        .call_tool(
            CallToolRequestParams::new("browser_upload").with_arguments(arguments(
                serde_json::json!({"workspace_id":ws.id,"sources":[{"kind":"workspace","path":"upload.txt"}]}),
            )),
        )
        .await
        .unwrap();
    assert!(
        !upload.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&upload).unwrap()
    );
    let upload_call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "browser_file_upload")
        .unwrap();
    let staged = upload_call.arguments["paths"][0].as_str().unwrap();
    assert!(staged.contains("upload-staging"));
    let staged_name = std::path::Path::new(staged)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(
        staged_name.starts_with("u-"),
        "staged upload must use an opaque id: {staged_name}"
    );
    assert!(
        !staged_name.contains("upload"),
        "staged upload leaks the original filename: {staged_name}"
    );

    // Real Playwright writes completed downloads into the session outputDir
    // root; discovery must stage them into downloads/ without test help.
    std::fs::write(driver.artifact_dir().join("result.txt"), b"download-body").unwrap();
    let downloads = client
        .call_tool(CallToolRequestParams::new("browser_downloads"))
        .await
        .unwrap();
    let download_id = downloads.structured_content.as_ref().unwrap()[0]["download_id"]
        .as_str()
        .unwrap()
        .to_string();
    let save = client.call_tool(CallToolRequestParams::new("browser_save_download_to_workspace").with_arguments(arguments(serde_json::json!({"workspace_id":ws.id,"download_id":download_id,"path":"saved/result.txt"})))).await.unwrap();
    assert!(
        !save.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&save).unwrap()
    );
    assert_eq!(
        std::fs::read(workspace_root.join("saved/result.txt")).unwrap(),
        b"download-body"
    );

    let steps = client.call_tool(CallToolRequestParams::new("browser_run_steps").with_arguments(arguments(serde_json::json!({"steps":[{"type":"navigate","url":"https://example.test/flow"},{"type":"snapshot"},{"type":"set_viewport","width":320,"height":568}]})))).await.unwrap();
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
        .contains("BROWSER_FEATURE_UNSUPPORTED_IN_MODE"));

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
    let (port, _dir, core, driver) = spawn(settings).await;
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
    let (port, _dir, core, _driver) = spawn(settings).await;
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
    let (port, _dir, core, _driver) = spawn(AppSettings::default()).await;
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
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
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

    assert!(driver
        .calls()
        .iter()
        .any(|call| { call.tool == "browser_navigate" && call.arguments["url"] == secret_url }));
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
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
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
async fn navigation_search_encodes_query_without_auditing_query_text() {
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
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
    let search_call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|call| call.tool == "browser_navigate")
        .expect("search must map to browser_navigate");
    let url = search_call.arguments["url"].as_str().unwrap();
    let parsed = url::Url::parse(url).unwrap();
    assert_eq!(parsed.scheme(), "https");
    assert_eq!(parsed.host_str(), Some("www.google.com"));
    assert_eq!(parsed.path(), "/search");
    assert_eq!(
        parsed.query_pairs().find(|(key, _)| key == "q").unwrap().1,
        query
    );

    let rows = matching_audit(&core, "browser_search");
    assert_eq!(rows.len(), 1);
    let detail = rows[0].detail.as_ref().unwrap();
    assert_eq!(detail["query_bytes"], query.len());
    assert!(!serde_json::to_string(detail).unwrap().contains(query));
}

#[tokio::test]
async fn lifecycle_tabs_use_opaque_page_ids_and_reconcile_indices_after_close() {
    let (port, _dir, _core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    let opened = client
        .call_tool(CallToolRequestParams::new("browser_open"))
        .await
        .unwrap();
    assert!(!opened.is_error.unwrap_or(false));
    {
        let mut tabs = driver.tabs.lock().unwrap();
        tabs[0].title = "New".into();
        tabs[0].url = "https://example.org/new".into();
    }

    let new_tab = client
        .call_tool(
            CallToolRequestParams::new("browser_tabs").with_arguments(arguments(
                serde_json::json!({"action":"new","url":"https://example.org/new"}),
            )),
        )
        .await
        .unwrap();
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

    let close = client
        .call_tool(
            CallToolRequestParams::new("browser_tabs").with_arguments(arguments(
                serde_json::json!({"action":"close","page_id":first_id.clone()}),
            )),
        )
        .await
        .unwrap();
    assert!(!close.is_error.unwrap_or(false));
    let close_call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|call| call.tool == "browser_tabs" && call.arguments["action"] == "close")
        .expect("close must reach browser_tabs");
    assert_eq!(close_call.arguments["index"], 0);

    let pages = close
        .structured_content
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0]["page_id"], second_id);

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
async fn interaction_tools_map_exactly_to_upstream_primitives() {
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    let cases: Vec<(&str, serde_json::Value, &str, serde_json::Value)> = vec![
        (
            "browser_snapshot",
            serde_json::json!({}),
            "browser_snapshot",
            serde_json::json!({}),
        ),
        (
            "browser_click",
            serde_json::json!({"ref":"e12"}),
            "browser_click",
            serde_json::json!({"target":"e12"}),
        ),
        (
            "browser_fill",
            serde_json::json!({"ref":"e3","text":"secret"}),
            "browser_type",
            serde_json::json!({"target":"e3","text":"secret","slowly":false,"submit":false}),
        ),
        (
            "browser_press",
            serde_json::json!({"key":"Enter"}),
            "browser_press_key",
            serde_json::json!({"key":"Enter"}),
        ),
        (
            "browser_select",
            serde_json::json!({"ref":"e7","values":["US"]}),
            "browser_select_option",
            serde_json::json!({"target":"e7","values":["US"]}),
        ),
        (
            "browser_hover",
            serde_json::json!({"ref":"e1"}),
            "browser_hover",
            serde_json::json!({"target":"e1"}),
        ),
        (
            "browser_drag",
            serde_json::json!({"start_ref":"e1","end_ref":"e2"}),
            "browser_drag",
            serde_json::json!({"startTarget":"e1","endTarget":"e2"}),
        ),
        (
            "browser_dialog",
            serde_json::json!({"accept":true}),
            "browser_handle_dialog",
            serde_json::json!({"accept":true}),
        ),
    ];
    for (tool, args, upstream, upstream_args) in cases {
        let result = client
            .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments(args)))
            .await
            .unwrap();
        assert!(
            !result.is_error.unwrap_or(false),
            "{tool}: {}",
            serde_json::to_string(&result).unwrap()
        );
        let call = driver
            .calls()
            .into_iter()
            .rev()
            .find(|c| c.tool == upstream)
            .unwrap_or_else(|| panic!("{tool} must map to {upstream}"));
        assert_eq!(call.arguments, upstream_args, "{tool} mapping changed");
    }

    for (action, upstream) in [
        ("back", "browser_navigate_back"),
        ("forward", "browser_navigate_forward"),
        ("reload", "browser_reload"),
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
        assert!(
            driver.calls().iter().any(|c| c.tool == upstream),
            "navigate {action} must map to {upstream}"
        );
    }

    let rows = matching_audit(&core, "browser_fill");
    assert_eq!(rows.len(), 1);
    let detail = rows[0].detail.as_ref().unwrap();
    assert_eq!(detail["target"], "e3");
    assert_eq!(detail["text_bytes"], 6);
    assert!(
        !serde_json::to_string(detail).unwrap().contains("secret"),
        "fill audit must store byte counts only"
    );
}

#[tokio::test]
async fn interaction_stale_ref_surfaces_stable_code_and_audits_error() {
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    driver.fail(
        "browser_click",
        CoreError::new(
            ErrorCode::BrowserStaleRef,
            "browser element reference is stale; take a new snapshot",
        ),
    );

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_click")
                .with_arguments(arguments(serde_json::json!({"ref":"e12"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(result.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_STALE_REF"), "{text}");

    let rows = matching_audit(&core, "browser_click");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, "error");
    assert_eq!(rows[0].error_code.as_deref(), Some("BROWSER_STALE_REF"));
}

#[tokio::test]
async fn snapshot_300kib_is_truncated_to_256kib_with_byte_only_audit() {
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    driver.respond("browser_snapshot", "x".repeat(300 * 1024));

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
async fn wait_conditions_map_to_upstream_primitives() {
    let (port, _dir, _core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    let ok_cases = [
        (
            serde_json::json!({"condition":{"kind":"text_present","text":"Ready"}}),
            serde_json::json!({"text":"Ready"}),
        ),
        (
            serde_json::json!({"condition":{"kind":"text_absent","text":"Gone"}}),
            serde_json::json!({"textGone":"Gone"}),
        ),
        (
            serde_json::json!({"condition":{"kind":"delay","milliseconds":250},"timeout_ms":2000}),
            serde_json::json!({"time":0.25}),
        ),
    ];
    for (args, expected) in ok_cases {
        let result = client
            .call_tool(CallToolRequestParams::new("browser_wait").with_arguments(arguments(args)))
            .await
            .unwrap();
        assert!(
            !result.is_error.unwrap_or(false),
            "{}",
            serde_json::to_string(&result).unwrap()
        );
        let call = driver
            .calls()
            .into_iter()
            .rev()
            .find(|c| c.tool == "browser_wait_for")
            .expect("text/delay waits must map to browser_wait_for");
        assert_eq!(call.arguments, expected);
    }

    let before = driver
        .calls()
        .iter()
        .filter(|c| c.tool == "browser_wait_for")
        .count();
    let too_long = client
        .call_tool(CallToolRequestParams::new("browser_wait").with_arguments(arguments(
            serde_json::json!({"condition":{"kind":"delay","milliseconds":5000},"timeout_ms":1000}),
        )))
        .await
        .unwrap();
    let text = serde_json::to_string(&too_long).unwrap();
    assert!(too_long.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("INVALID_ARGUMENT"), "{text}");
    assert_eq!(
        driver
            .calls()
            .iter()
            .filter(|c| c.tool == "browser_wait_for")
            .count(),
        before,
        "over-long delay reached the driver"
    );

    let poll_cases = [
        (
            serde_json::json!({"condition":{"kind":"ref_visible","ref_id":"e9"},"timeout_ms":2000}),
            "getBoundingClientRect",
            Some("e9"),
        ),
        (
            serde_json::json!({"condition":{"kind":"url_contains","value":"example.test"},"timeout_ms":2000}),
            "location.href",
            None,
        ),
        (
            serde_json::json!({"condition":{"kind":"load_state","state":"complete"},"timeout_ms":2000}),
            "document.readyState",
            None,
        ),
    ];
    for (args, marker, target) in poll_cases {
        let result = client
            .call_tool(CallToolRequestParams::new("browser_wait").with_arguments(arguments(args)))
            .await
            .unwrap();
        assert!(
            !result.is_error.unwrap_or(false),
            "{}",
            serde_json::to_string(&result).unwrap()
        );
        let call = driver
            .calls()
            .into_iter()
            .rev()
            .find(|c| {
                c.tool == "browser_evaluate"
                    && c.arguments["function"]
                        .as_str()
                        .is_some_and(|f| f.contains(marker))
            })
            .unwrap_or_else(|| panic!("wait must poll browser_evaluate with {marker}"));
        match target {
            Some(expected) => assert_eq!(call.arguments["target"], expected),
            None => assert!(call.arguments.get("target").is_none()),
        }
    }

    assert!(
        driver
            .calls()
            .iter()
            .all(|c| c.tool != "browser_run_code" && c.tool != "browser_run_code_unsafe"),
        "wait polling must never use process-level code execution"
    );
}

#[tokio::test]
async fn viewport_table_maps_to_browser_resize_and_invalid_is_rejected_before_driver() {
    let (port, _dir, _core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    for (width, height) in [(320u32, 568u32), (375, 812), (768, 1024), (1440, 900)] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("browser_set_viewport").with_arguments(arguments(
                    serde_json::json!({"width": width, "height": height}),
                )),
            )
            .await
            .unwrap();
        assert!(
            !result.is_error.unwrap_or(false),
            "{width}x{height}: {}",
            serde_json::to_string(&result).unwrap()
        );
        let call = driver
            .calls()
            .into_iter()
            .rev()
            .find(|c| c.tool == "browser_resize")
            .expect("viewport must map to browser_resize");
        assert_eq!(
            call.arguments,
            serde_json::json!({"width": width, "height": height})
        );
    }
    let resize_calls = driver
        .calls()
        .iter()
        .filter(|c| c.tool == "browser_resize")
        .count();
    assert_eq!(resize_calls, 4);

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
        driver
            .calls()
            .iter()
            .filter(|c| c.tool == "browser_resize")
            .count(),
        4,
        "invalid viewport reached the driver"
    );
}

#[tokio::test]
async fn device_restart_reuses_profile_starts_once_and_never_replays() {
    let (port, _dir, _core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    let before_session = client
        .call_tool(CallToolRequestParams::new("browser_get_status"))
        .await
        .unwrap()
        .structured_content
        .unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    client
        .call_tool(
            CallToolRequestParams::new("browser_navigate").with_arguments(arguments(
                serde_json::json!({"action":"url","url":"https://example.test/before"}),
            )),
        )
        .await
        .unwrap();
    let navigates_before = driver
        .calls()
        .iter()
        .filter(|c| c.tool == "browser_navigate")
        .count();

    let emulate = client
        .call_tool(
            CallToolRequestParams::new("browser_emulate_device").with_arguments(arguments(
                serde_json::json!({"emulation":{"source":"preset","name":"Test Phone"}}),
            )),
        )
        .await
        .unwrap();
    assert!(
        !emulate.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&emulate).unwrap()
    );
    let after_session = emulate.structured_content.as_ref().unwrap()["session_id"]
        .as_str()
        .unwrap();
    assert_ne!(
        before_session, after_session,
        "emulation must start a new session"
    );
    assert_eq!(driver.stops.load(Ordering::SeqCst), 1, "exactly one stop");
    assert_eq!(
        driver.starts.load(Ordering::SeqCst),
        2,
        "exactly one restart"
    );

    let launches = driver.launches();
    assert_eq!(launches.len(), 2);
    assert_eq!(
        launches[0].profile_dir, launches[1].profile_dir,
        "the same engine profile must be reused"
    );
    let emulation = launches[1]
        .emulation
        .as_ref()
        .expect("restart launch must carry emulation");
    assert_eq!((emulation.width, emulation.height), (390, 844));
    assert_eq!(emulation.device_scale_factor, 3.0);
    assert!(emulation.is_mobile);
    assert!(emulation.has_touch);
    assert_eq!(emulation.user_agent.as_deref(), Some("TestPhone/1.0"));

    assert_eq!(
        driver
            .calls()
            .iter()
            .filter(|c| c.tool == "browser_navigate")
            .count(),
        navigates_before,
        "old interactions were replayed after restart"
    );
}

#[tokio::test]
async fn screenshot_modes_metadata_and_os_capture_independence() {
    let settings = AppSettings {
        screen_capture_enabled: false,
        ..AppSettings::default()
    };
    let (port, _dir, _core, driver) = spawn(settings).await;
    let client = client(port).await;
    open_browser(&client).await;

    // OS Screen Capture is explicitly off; browser screenshots still succeed.
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_screenshot").with_arguments(arguments(
                serde_json::json!({"mode":"viewport","max_edge":2}),
            )),
        )
        .await
        .unwrap();
    assert!(
        !result.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&result).unwrap()
    );
    assert!(result
        .content
        .iter()
        .any(|block| block.as_image().is_some()));
    let metadata = result.structured_content.as_ref().unwrap().clone();
    assert_eq!(metadata["mode"], "viewport");
    assert_eq!(metadata["original_width"], 4);
    assert_eq!(metadata["original_height"], 3);
    assert_eq!(metadata["returned_width"], 2);
    assert!(metadata["encoded_bytes"].as_u64().unwrap() > 0);
    assert!(metadata["sha256"].as_str().unwrap().len() == 64);

    // Element mode forwards the ref as the upstream target.
    let element = client
        .call_tool(
            CallToolRequestParams::new("browser_screenshot")
                .with_arguments(arguments(serde_json::json!({"mode":"element","ref":"e5"}))),
        )
        .await
        .unwrap();
    assert!(!element.is_error.unwrap_or(false));
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "browser_take_screenshot")
        .unwrap();
    assert_eq!(call.arguments["target"], "e5");
    assert_eq!(call.arguments["type"], "png");
    assert_eq!(call.arguments["fullPage"], false);

    let full = client
        .call_tool(
            CallToolRequestParams::new("browser_screenshot")
                .with_arguments(arguments(serde_json::json!({"mode":"full_page"}))),
        )
        .await
        .unwrap();
    assert!(!full.is_error.unwrap_or(false));
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "browser_take_screenshot")
        .unwrap();
    assert_eq!(call.arguments["fullPage"], true);
    assert!(call.arguments.get("target").is_none());
}

#[tokio::test]
async fn screenshot_ref_validation_rejects_wrong_mode_combinations() {
    let (port, _dir, _core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    let missing = client
        .call_tool(
            CallToolRequestParams::new("browser_screenshot")
                .with_arguments(arguments(serde_json::json!({"mode":"element"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&missing).unwrap();
    assert!(missing.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("INVALID_ARGUMENT"), "{text}");

    for mode in ["viewport", "full_page"] {
        let extra = client
            .call_tool(
                CallToolRequestParams::new("browser_screenshot")
                    .with_arguments(arguments(serde_json::json!({"mode": mode, "ref":"e1"}))),
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&extra).unwrap();
        assert!(extra.is_error.unwrap_or(false), "{mode}: {text}");
        assert!(text.contains("INVALID_ARGUMENT"), "{mode}: {text}");
    }
    assert!(
        driver
            .calls()
            .iter()
            .all(|c| c.tool != "browser_take_screenshot"),
        "invalid screenshot mode/ref reached the driver"
    );
}

fn png_header_block(width: u32, height: u32) -> rmcp::model::ContentBlock {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8u8, 6, 0, 0, 0]);
    rmcp::model::ContentBlock::image(
        base64::engine::general_purpose::STANDARD.encode(bytes),
        "image/png",
    )
}

#[tokio::test]
async fn screenshot_resource_bounds_reject_pathological_images() {
    let (port, _dir, _core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    // 10^10 pixels exceeds the pixel budget and must be rejected from the PNG
    // header before any decode allocates memory.
    *driver.screenshot_block.lock().unwrap() = Some(png_header_block(100_000, 100_000));
    let pixels = client
        .call_tool(
            CallToolRequestParams::new("browser_screenshot")
                .with_arguments(arguments(serde_json::json!({"mode":"viewport"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&pixels).unwrap();
    assert!(pixels.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("pixel budget"), "{text}");

    // A 33 MiB encoded payload exceeds the encoded-byte budget.
    let big = vec![0u8; 33 * 1024 * 1024];
    *driver.screenshot_block.lock().unwrap() = Some(rmcp::model::ContentBlock::image(
        base64::engine::general_purpose::STANDARD.encode(big),
        "image/png",
    ));
    let encoded = client
        .call_tool(
            CallToolRequestParams::new("browser_screenshot")
                .with_arguments(arguments(serde_json::json!({"mode":"viewport"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&encoded).unwrap();
    assert!(encoded.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("encoded byte budget"), "{text}");
}

#[tokio::test]
async fn console_cursor_levels_and_audit_metadata() {
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    // Pinned @playwright/mcp 0.0.79 console shape: header line, blank line,
    // then `[LEVEL] text @ url:line` rows.
    let script = "Total messages: 5 (Errors: 1, Warnings: 1)\n\n[INFO] one @ https://example.test/app.js:1\n[WARNING] two @ https://example.test/app.js:2\n[ERROR] three @ https://example.test/app.js:3\n[INFO] four @ https://example.test/app.js:4\n[DEBUG] five @ https://example.test/app.js:5";
    for _ in 0..3 {
        driver.respond("browser_console_messages", script.to_string());
    }

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
    assert_eq!(
        structured["messages"],
        "[INFO] one @ https://example.test/app.js:1\n[WARNING] two @ https://example.test/app.js:2"
    );
    assert_eq!(structured["levels"]["info"], 1);
    assert_eq!(structured["levels"]["warning"], 1);
    assert!(
        !structured["messages"]
            .as_str()
            .unwrap()
            .contains("Total messages"),
        "upstream presentation header must not consume the first page"
    );
    let cursor1 = structured["next_cursor"].as_str().unwrap().to_string();

    let page2 = client
        .call_tool(
            CallToolRequestParams::new("browser_console")
                .with_arguments(arguments(serde_json::json!({"limit":2,"cursor":cursor1}))),
        )
        .await
        .unwrap();
    let structured = page2.structured_content.as_ref().unwrap().clone();
    assert_eq!(
        structured["messages"],
        "[ERROR] three @ https://example.test/app.js:3\n[INFO] four @ https://example.test/app.js:4"
    );
    let cursor2 = structured["next_cursor"].as_str().unwrap().to_string();

    let page3 = client
        .call_tool(
            CallToolRequestParams::new("browser_console")
                .with_arguments(arguments(serde_json::json!({"limit":2,"cursor":cursor2}))),
        )
        .await
        .unwrap();
    let structured = page3.structured_content.as_ref().unwrap().clone();
    assert_eq!(
        structured["messages"],
        "[DEBUG] five @ https://example.test/app.js:5"
    );
    assert!(structured["next_cursor"].is_null());

    // Audit stores only count/levels/cursor, never message text.
    let rows = matching_audit(&core, "browser_console");
    assert_eq!(rows.len(), 3);
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

    driver.respond(
        "browser_console_messages",
        "Total messages: 1 (Errors: 1, Warnings: 0)\n\n[ERROR] only @ https://example.test/app.js:1"
            .to_string(),
    );
    client
        .call_tool(
            CallToolRequestParams::new("browser_console")
                .with_arguments(arguments(serde_json::json!({"level":"error"}))),
        )
        .await
        .unwrap();
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "browser_console_messages")
        .unwrap();
    assert_eq!(call.arguments["level"], "error");
}

#[tokio::test]
async fn network_cursor_filter_and_redaction() {
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    // Pinned @playwright/mcp 0.0.79 network shape: numbered
    // `N. [METHOD] URL => [STATUS] STATUS_TEXT` rows. The sensitive header
    // lines are not part of real list output but are kept so redaction stays
    // covered defense-in-depth.
    let script = "1. [GET] https://a.test/ => [200] OK\nAuthorization: Bearer secret\n2. [GET] https://b.test/ => [200] OK\nCookie: session=secret\n3. [GET] https://c.test/ => [200] OK";
    driver.respond("browser_network_requests", script.to_string());
    driver.respond("browser_network_requests", script.to_string());

    let page1 = client
        .call_tool(
            CallToolRequestParams::new("browser_network").with_arguments(arguments(
                serde_json::json!({"limit":2,"filter":"api","include_static":true}),
            )),
        )
        .await
        .unwrap();
    assert!(!page1.is_error.unwrap_or(false));
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "browser_network_requests")
        .unwrap();
    assert_eq!(call.arguments["filter"], "api");
    assert_eq!(call.arguments["static"], true);
    let structured = page1.structured_content.as_ref().unwrap().clone();
    assert_eq!(structured["count"], 2);
    let requests = structured["requests"].as_str().unwrap();
    assert!(requests.contains("[GET] https://a.test/ => [200]"));
    assert!(requests.contains("Authorization: [redacted]"));
    assert!(!requests.contains("Bearer secret"));
    let cursor = structured["next_cursor"].as_str().unwrap().to_string();

    let page2 = client
        .call_tool(
            CallToolRequestParams::new("browser_network")
                .with_arguments(arguments(serde_json::json!({"limit":10,"cursor":cursor}))),
        )
        .await
        .unwrap();
    let structured = page2.structured_content.as_ref().unwrap().clone();
    let requests = structured["requests"].as_str().unwrap();
    assert!(requests.contains("Cookie: [redacted]"));
    assert!(!requests.contains("session=secret"));
    assert!(structured["next_cursor"].is_null());

    let rows = matching_audit(&core, "browser_network");
    assert_eq!(rows.len(), 2);
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

    // The filter is a regular expression; length/control-character guards
    // reject bad filters before any upstream call is made.
    let upstream_calls = driver
        .calls()
        .iter()
        .filter(|c| c.tool == "browser_network_requests")
        .count();
    for bad in [
        serde_json::json!({"filter":"a\nb"}),
        serde_json::json!({"filter":"x".repeat(4097)}),
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
            .filter(|c| c.tool == "browser_network_requests")
            .count(),
        upstream_calls,
        "rejected filters must not reach upstream"
    );
}

#[tokio::test]
async fn dom_inspection_uses_fixed_function_with_exact_ref() {
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_dom")
                .with_arguments(arguments(serde_json::json!({"ref":"e2"}))),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "browser_evaluate")
        .expect("browser_dom must map to browser_evaluate");
    assert_eq!(call.arguments["target"], "e2");
    let function = call.arguments["function"].as_str().unwrap();
    assert!(
        function.contains("cloneNode(true)"),
        "must serialize a clone"
    );
    assert!(
        function.contains("[redacted]"),
        "must redact sensitive values"
    );
    assert!(function.contains("outerHTML"));
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
async fn evaluate_maps_to_page_context_and_never_process_code() {
    let (port, _dir, _core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_evaluate").with_arguments(arguments(
                serde_json::json!({"expression":"() => 42","ref":"e1"}),
            )),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "browser_evaluate")
        .unwrap();
    assert_eq!(call.arguments["function"], "() => 42");
    assert_eq!(call.arguments["target"], "e1");

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

    assert!(
        driver
            .calls()
            .iter()
            .all(|c| c.tool != "browser_run_code" && c.tool != "browser_run_code_unsafe"),
        "evaluate must never route through process-level code execution"
    );
}

#[tokio::test]
async fn performance_merges_cdp_for_chromium_and_degrades_for_firefox() {
    {
        let (port, _dir, _core, driver) = spawn(AppSettings::default()).await;
        let client = client(port).await;
        open_browser(&client).await;

        let result = client
            .call_tool(CallToolRequestParams::new("browser_performance"))
            .await
            .unwrap();
        assert!(!result.is_error.unwrap_or(false));
        let structured = result.structured_content.as_ref().unwrap().clone();
        assert_eq!(structured["cdp_supported"], true);
        assert_eq!(structured["cdp"], serde_json::json!({"metrics": []}));
        assert_eq!(driver.cdp_calls.load(Ordering::SeqCst), 1);
        assert!(driver.calls().iter().any(|c| {
            c.tool == "browser_evaluate"
                && c.arguments["function"]
                    .as_str()
                    .is_some_and(|f| f.contains("performance.getEntriesByType"))
        }));
    }

    let firefox_settings = AppSettings {
        browser_engine: BrowserEngine::Firefox,
        ..AppSettings::default()
    };
    let (port, _dir, _core, driver) = spawn(firefox_settings).await;
    let firefox_client = client(port).await;
    open_browser(&firefox_client).await;
    let result = firefox_client
        .call_tool(CallToolRequestParams::new("browser_performance"))
        .await
        .unwrap();
    assert!(
        !result.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&result).unwrap()
    );
    let structured = result.structured_content.as_ref().unwrap().clone();
    assert_eq!(structured["cdp_supported"], false);
    assert!(structured["cdp"].is_null());
    assert_eq!(driver.cdp_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn trace_lifecycle_returns_opaque_artifacts_without_paths() {
    let (port, _dir, _core, driver) = spawn(AppSettings::default()).await;
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
    assert!(driver
        .calls()
        .iter()
        .any(|c| c.tool == "browser_start_tracing"));

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
    assert_eq!(structured["files"], 1, "{text}");
    assert_eq!(structured["bytes"], 5, "{text}");
    assert!(structured.get("artifacts").is_none(), "{text}");
    assert!(
        !text.contains("trace.zip"),
        "trace response leaks file names"
    );
    let artifact_dir = driver.artifact_dir();
    let dir_string = artifact_dir.to_string_lossy().to_string();
    assert!(
        !text.contains(&dir_string),
        "trace response leaks absolute paths"
    );
    assert!(
        !text.contains(&dir_string.replace('\\', "/")),
        "trace response leaks absolute paths"
    );

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
async fn devtools_reports_launch_time_state_truthfully() {
    {
        let visible_firefox = AppSettings {
            browser_run_mode: BrowserRunMode::VisibleDebug,
            browser_engine: BrowserEngine::Firefox,
            ..AppSettings::default()
        };
        let (port, _dir, _core, _driver) = spawn(visible_firefox).await;
        let client = client(port).await;
        open_browser(&client).await;
        let result = client
            .call_tool(CallToolRequestParams::new("browser_devtools"))
            .await
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(result.is_error.unwrap_or(false), "{text}");
        assert!(text.contains("BROWSER_FEATURE_UNSUPPORTED"), "{text}");
    }

    let visible_chromium = AppSettings {
        browser_run_mode: BrowserRunMode::VisibleDebug,
        ..AppSettings::default()
    };
    let (port, _dir, _core, driver) = spawn(visible_chromium).await;
    let chromium_client = client(port).await;
    open_browser(&chromium_client).await;
    let result = chromium_client
        .call_tool(CallToolRequestParams::new("browser_devtools"))
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(!result.is_error.unwrap_or(false), "{text}");
    let structured = result.structured_content.as_ref().unwrap().clone();
    assert_eq!(structured["supported"], true);
    assert_eq!(structured["mode"], "visible_debug");
    assert_eq!(structured["auto_open"], true);
    assert_eq!(
        driver.starts.load(Ordering::SeqCst),
        1,
        "devtools must never launch a second browser"
    );
}

#[tokio::test]
async fn upload_deny_allow_table() {
    let (port, dir, core, driver) = spawn(AppSettings::default()).await;
    let workspace_root = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace_root).unwrap();
    std::fs::write(workspace_root.join("upload.txt"), b"upload-body").unwrap();
    let locked_root = dir.path().join("locked");
    std::fs::create_dir_all(&locked_root).unwrap();
    std::fs::write(locked_root.join("locked.txt"), b"locked-body").unwrap();
    let ws = lc_core::workspace_repo::insert_workspace(
        &core.db,
        "Web",
        &workspace_root.to_string_lossy(),
    )
    .unwrap();
    lc_core::workspace_repo::update_capabilities(&core.db, &ws.id, true, true).unwrap();
    let locked = lc_core::workspace_repo::insert_workspace(
        &core.db,
        "Locked",
        &locked_root.to_string_lossy(),
    )
    .unwrap();
    lc_core::workspace_repo::update_capabilities(&core.db, &locked.id, false, false).unwrap();

    let client = client(port).await;
    open_browser(&client).await;

    // Allowed: workspace source with Read permission.
    let ok = client
        .call_tool(CallToolRequestParams::new("browser_upload").with_arguments(arguments(
            serde_json::json!({"workspace_id":ws.id,"sources":[{"kind":"workspace","path":"upload.txt"}]}),
        )))
        .await
        .unwrap();
    assert!(
        !ok.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&ok).unwrap()
    );
    let call = driver
        .calls()
        .into_iter()
        .rev()
        .find(|c| c.tool == "browser_file_upload")
        .unwrap();
    let staged = call.arguments["paths"][0].as_str().unwrap();
    assert!(staged.contains("upload-staging"));

    // Denied: workspace without Read permission.
    let denied = client
        .call_tool(CallToolRequestParams::new("browser_upload").with_arguments(arguments(
            serde_json::json!({"workspace_id":locked.id,"sources":[{"kind":"workspace","path":"locked.txt"}]}),
        )))
        .await
        .unwrap();
    let text = serde_json::to_string(&denied).unwrap();
    assert!(denied.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_UPLOAD_DENIED"), "{text}");

    // Denied: traversal and absolute paths.
    for path in ["../outside.txt", "C:/evil.txt"] {
        let denied = client
            .call_tool(CallToolRequestParams::new("browser_upload").with_arguments(arguments(
                serde_json::json!({"workspace_id":ws.id,"sources":[{"kind":"workspace","path":path}]}),
            )))
            .await
            .unwrap();
        let text = serde_json::to_string(&denied).unwrap();
        assert!(denied.is_error.unwrap_or(false), "{path}: {text}");
        assert!(text.contains("BROWSER_UPLOAD_DENIED"), "{path}: {text}");
    }

    // Denied: workspace source without workspace_id.
    let missing_ws = client
        .call_tool(
            CallToolRequestParams::new("browser_upload").with_arguments(arguments(
                serde_json::json!({"sources":[{"kind":"workspace","path":"upload.txt"}]}),
            )),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&missing_ws).unwrap();
    assert!(missing_ws.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("INVALID_ARGUMENT"), "{text}");

    // Allowed: artifact id resolving inside the active session sandbox.
    std::fs::write(
        driver.artifact_dir().join("artifact-src.txt"),
        b"artifact-body",
    )
    .unwrap();
    let downloads = client
        .call_tool(CallToolRequestParams::new("browser_downloads"))
        .await
        .unwrap();
    let artifact_id = downloads.structured_content.as_ref().unwrap()[0]["download_id"]
        .as_str()
        .unwrap()
        .to_string();
    let from_artifact = client
        .call_tool(
            CallToolRequestParams::new("browser_upload").with_arguments(arguments(
                serde_json::json!({"sources":[{"kind":"artifact","artifact_id":artifact_id}]}),
            )),
        )
        .await
        .unwrap();
    assert!(
        !from_artifact.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&from_artifact).unwrap()
    );

    // Denied: unknown artifact id.
    let bogus = client
        .call_tool(CallToolRequestParams::new("browser_upload").with_arguments(arguments(
            serde_json::json!({"sources":[{"kind":"artifact","artifact_id":"d-0000000000000000-4"}]}),
        )))
        .await
        .unwrap();
    let text = serde_json::to_string(&bogus).unwrap();
    assert!(bogus.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_UPLOAD_DENIED"), "{text}");

    // Audit records source kinds and never file content.
    let rows = matching_audit(&core, "browser_upload");
    let allowed = rows
        .iter()
        .find(|r| {
            r.decision == "allow"
                && r.detail
                    .as_ref()
                    .is_some_and(|d| d["sources"][0]["kind"] == "workspace")
        })
        .expect("workspace upload must audit as allow");
    let detail = allowed.detail.as_ref().unwrap();
    assert_eq!(detail["sources"][0]["kind"], "workspace");
    assert_eq!(detail["sources"][0]["path"], "upload.txt");
    assert!(
        !serde_json::to_string(detail)
            .unwrap()
            .contains("upload-body"),
        "upload audit leaks file content"
    );
    assert!(
        rows.iter().any(|r| {
            r.decision == "allow"
                && r.detail
                    .as_ref()
                    .is_some_and(|d| d["sources"][0]["kind"] == "artifact")
        }),
        "artifact upload must audit as allow"
    );
    assert!(rows.iter().any(|r| r.decision == "deny"));
}

#[tokio::test]
async fn download_discovery_export_deny_allow_table() {
    let (port, dir, core, driver) = spawn(AppSettings::default()).await;
    let workspace_root = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let ws = lc_core::workspace_repo::insert_workspace(
        &core.db,
        "Web",
        &workspace_root.to_string_lossy(),
    )
    .unwrap();
    lc_core::workspace_repo::update_capabilities(&core.db, &ws.id, true, true).unwrap();
    let readonly_root = dir.path().join("readonly");
    std::fs::create_dir_all(&readonly_root).unwrap();
    let readonly = lc_core::workspace_repo::insert_workspace(
        &core.db,
        "ReadOnly",
        &readonly_root.to_string_lossy(),
    )
    .unwrap();
    lc_core::workspace_repo::update_capabilities(&core.db, &readonly.id, true, false).unwrap();

    let client = client(port).await;
    open_browser(&client).await;

    // Real Playwright writes completed downloads into the outputDir root and
    // in-flight downloads as .part files under downloads/.
    std::fs::write(driver.artifact_dir().join("result.txt"), b"download-body").unwrap();
    std::fs::create_dir_all(driver.artifact_dir().join("downloads")).unwrap();
    std::fs::write(
        driver
            .artifact_dir()
            .join("downloads")
            .join("busy.txt.part"),
        b"partial",
    )
    .unwrap();

    let downloads = client
        .call_tool(CallToolRequestParams::new("browser_downloads"))
        .await
        .unwrap();
    let text = serde_json::to_string(&downloads).unwrap();
    assert!(!downloads.is_error.unwrap_or(false), "{text}");
    let list = downloads
        .structured_content
        .as_ref()
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(list.len(), 2);
    let done = list.iter().find(|d| d["filename"] == "result.txt").unwrap();
    let busy = list.iter().find(|d| d["filename"] == "busy.txt").unwrap();
    assert!(done["download_id"].as_str().unwrap().starts_with("d-"));
    assert_eq!(done["completed"], true);
    assert!(done["sha256"].as_str().unwrap().len() == 64);
    assert_eq!(busy["completed"], false);
    assert!(
        busy["sha256"].is_null(),
        "incomplete downloads must have no hash"
    );
    let artifact_dir = driver.artifact_dir();
    let dir_string = artifact_dir.to_string_lossy().to_string();
    assert!(!text.contains(&dir_string), "downloads leak absolute paths");

    let done_id = done["download_id"].as_str().unwrap().to_string();
    let busy_id = busy["download_id"].as_str().unwrap().to_string();

    // Denied: exporting an incomplete download.
    let incomplete = client.call_tool(CallToolRequestParams::new("browser_save_download_to_workspace").with_arguments(arguments(serde_json::json!({"workspace_id":ws.id,"download_id":busy_id,"path":"busy.txt"})))).await.unwrap();
    let text = serde_json::to_string(&incomplete).unwrap();
    assert!(incomplete.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_DOWNLOAD_DENIED"), "{text}");

    // Denied: unknown id.
    let missing = client.call_tool(CallToolRequestParams::new("browser_save_download_to_workspace").with_arguments(arguments(serde_json::json!({"workspace_id":ws.id,"download_id":"d-0000000000000000-1","path":"x.txt"})))).await.unwrap();
    let text = serde_json::to_string(&missing).unwrap();
    assert!(missing.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_DOWNLOAD_NOT_FOUND"), "{text}");

    // Denied: traversal destination.
    let traversal = client.call_tool(CallToolRequestParams::new("browser_save_download_to_workspace").with_arguments(arguments(serde_json::json!({"workspace_id":ws.id,"download_id":done_id,"path":"../evil.txt"})))).await.unwrap();
    let text = serde_json::to_string(&traversal).unwrap();
    assert!(traversal.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_DOWNLOAD_DENIED"), "{text}");

    // Denied: read-only workspace.
    let no_write = client.call_tool(CallToolRequestParams::new("browser_save_download_to_workspace").with_arguments(arguments(serde_json::json!({"workspace_id":readonly.id,"download_id":done_id,"path":"result.txt"})))).await.unwrap();
    let text = serde_json::to_string(&no_write).unwrap();
    assert!(no_write.is_error.unwrap_or(false), "{text}");
    assert!(text.contains("BROWSER_DOWNLOAD_DENIED"), "{text}");

    // Allowed: confined export with Write permission.
    let save = client.call_tool(CallToolRequestParams::new("browser_save_download_to_workspace").with_arguments(arguments(serde_json::json!({"workspace_id":ws.id,"download_id":done_id,"path":"saved/result.txt"})))).await.unwrap();
    assert!(
        !save.is_error.unwrap_or(false),
        "{}",
        serde_json::to_string(&save).unwrap()
    );
    let structured = save.structured_content.as_ref().unwrap().clone();
    assert_eq!(structured["workspace_id"], ws.id);
    assert_eq!(structured["path"], "saved/result.txt");
    assert_eq!(structured["bytes"], 13);
    assert!(structured["sha256"].as_str().unwrap().len() == 64);
    assert_eq!(
        std::fs::read(workspace_root.join("saved/result.txt")).unwrap(),
        b"download-body"
    );
}

#[tokio::test]
async fn run_steps_rejects_over_limit_before_driver() {
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
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
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    driver.fail(
        "browser_click",
        CoreError::new(ErrorCode::BrowserStaleRef, "stale reference"),
    );

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_run_steps").with_arguments(arguments(
                serde_json::json!({"steps":[
                    {"type":"navigate","url":"https://example.test/flow"},
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
    assert_eq!(structured["failed_index"], 2);
    assert_eq!(structured["status"], "failed");
    assert_eq!(structured["error"]["code"], "BROWSER_STALE_REF");
    assert_eq!(structured["results"].as_array().unwrap().len(), 2);
    assert!(
        driver.calls().iter().all(|c| c.tool != "browser_snapshot"),
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
    assert_eq!(detail["step_count"], 4);
    assert_eq!(detail["failed_index"], 2);
    assert_eq!(detail["status"], "failed");
    assert_eq!(
        detail["step_types"],
        serde_json::json!(["navigate", "fill", "click", "snapshot"])
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
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
    let client = client(port).await;
    open_browser(&client).await;
    driver.fail(
        "browser_click",
        CoreError::new(ErrorCode::BrowserProcessLost, "browser process was lost"),
    );

    let result = client
        .call_tool(
            CallToolRequestParams::new("browser_run_steps").with_arguments(arguments(
                serde_json::json!({"steps":[{"type":"click","ref_id":"e1"}]}),
            )),
        )
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap().clone();
    assert_eq!(structured["ok"], false);
    assert_eq!(structured["failed_index"], 0);
    assert_eq!(structured["error"]["code"], "BROWSER_PROCESS_LOST");
    assert_eq!(
        driver
            .calls()
            .iter()
            .filter(|c| c.tool == "browser_click")
            .count(),
        1,
        "failed step was replayed"
    );
    let rows = matching_audit(&core, "browser_run_steps");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, "error");
}

#[tokio::test]
async fn run_steps_rejects_disallowed_step_types() {
    let (port, _dir, core, driver) = spawn(AppSettings::default()).await;
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
        driver.calls().iter().all(|c| c.tool != "browser_evaluate"
            && c.tool != "browser_file_upload"
            && c.tool != "browser_handle_dialog"
            && c.tool != "browser_start_tracing"),
        "disallowed step reached the driver"
    );
    // Protocol-level rejections happen before the handler: no audit rows.
    assert!(matching_audit(&core, "browser_run_steps").is_empty());
}
