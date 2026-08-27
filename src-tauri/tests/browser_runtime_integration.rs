//! Real bundled-browser integration tests (Task 14).
//!
//! Unlike `browser_mcp_integration.rs` (which drives a fake driver), this suite
//! drives the REAL production path end-to-end:
//!
//! ```text
//! MCP client -> WorkspaceServer handlers -> BrowserManager
//!   -> PlaywrightMcpDriver -> bundled Playwright MCP child process
//!   -> bundled Chromium / Firefox
//! ```
//!
//! The deterministic localhost fixture lives in `tests/fixtures/browser_test_app.rs`.
//!
//! These tests require the staged browser runtime at
//! `src-tauri/browser-runtime/runtime-manifest.json`. When that manifest is
//! absent (a checkout that has not run `npm run build:browser-runtime`), each
//! test prints an explicit SKIP message and passes, so the suite never fails
//! merely because the runtime is not staged. Release verification always stages
//! the runtime first, so these tests run for real there.
//!
//! Run with:
//! ```text
//! cargo test -p local-workspace-mcp-manager --test browser_runtime_integration --jobs 1 -- --nocapture
//! ```

#[path = "fixtures/browser_test_app.rs"]
mod browser_test_app;

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use base64::Engine as _;
use image::{GenericImageView as _, ImageFormat};
use serde_json::{json, Value};

use lc_core::{error::ErrorCode, CoreServices};
use local_workspace_mcp_manager::{
    browser::{
        manager::BrowserManager, playwright::PlaywrightMcpDriver, runtime::BrowserRuntimeManager,
    },
    screen_capture::{CaptureTarget, CapturedImage, ScreenCaptureProvider},
    server::WorkspaceServer,
    settings::{save_app_settings, AgentPolicyState, AgentSettingsController, AppSettings},
};
use rmcp::{
    model::{CallToolRequestParams, CallToolResult},
    service::{RoleClient, RunningService},
    transport::StreamableHttpClientTransport,
    ServiceExt,
};

use browser_test_app::{Fixture, DOWNLOAD_BODY};

type Client = RunningService<RoleClient, ()>;

/// Serialize real-browser tests. Each launches an actual browser; running them
/// in parallel would contend for CPU/GPU and make window/process-tree
/// assertions noisy. `--jobs 1` serializes test *binaries*, not the threads
/// inside one binary, so we serialize explicitly here.
static SERIAL: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

// ---------------------------------------------------------------------------
// Runtime discovery / skip gating
// ---------------------------------------------------------------------------

fn runtime_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("browser-runtime")
}

fn runtime_manifest() -> PathBuf {
    runtime_root().join("runtime-manifest.json")
}

/// Returns true when the bundled browser runtime is staged. Otherwise prints an
/// explicit skip message and returns false; callers `return` early so the test
/// passes with a deterministic skipped-unavailable condition.
///
/// During the Lightpanda native MCP migration this Playwright real-runtime
/// suite is superseded: `BrowserManager` no longer drives the Playwright
/// runtime, and Task 14 replaces this file with the WSL2/Lightpanda
/// real-runtime suite. Every case is a documented skip until then so the tree
/// stays green without ever faking a pass on the detached path.
fn runtime_available(case: &str) -> bool {
    if !runtime_manifest().is_file() {
        println!(
            "SKIP [{case}]: bundled browser runtime manifest absent at {}; \
             run `npm run build:browser-runtime` to stage it. Real-browser test skipped.",
            runtime_manifest().display()
        );
        return false;
    }
    println!(
        "SKIP [{case}]: Playwright real-runtime suite is superseded by the Lightpanda WSL2 \
         suite (Task 14); skipped during migration."
    );
    false
}

// ---------------------------------------------------------------------------
// Screen-capture stub (browser tests never use desktop screen capture)
// ---------------------------------------------------------------------------

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
        unreachable!("browser tests never capture the desktop screen")
    }
}

// ---------------------------------------------------------------------------
// Real-server harness
// ---------------------------------------------------------------------------

struct RealEnv {
    client: Client,
    driver: Arc<PlaywrightMcpDriver>,
    core: Arc<CoreServices>,
    app_data_dir: PathBuf,
    _dir: tempfile::TempDir,
}

/// Spawns the full production MCP HTTP server. During the Lightpanda migration
/// the manager is wired exactly like production (`with_lightpanda`); the
/// Playwright driver handle is kept only so the skipped test bodies still
/// compile until Task 14 rewrites this suite.
async fn spawn_real(settings: AppSettings) -> RealEnv {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_data_dir = dir.path().to_path_buf();
    save_app_settings(dir.path(), &settings).expect("save app settings");
    let core = Arc::new(CoreServices::open(&dir.path().join("test.db")).expect("open core"));
    let policy = Arc::new(AgentPolicyState::from_settings(&settings));
    let controller = Arc::new(AgentSettingsController::new(
        dir.path().to_path_buf(),
        policy.clone(),
    ));
    let driver = Arc::new(PlaywrightMcpDriver::new(runtime_root()));
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("lightpanda-runtime-manifest.json");
    let manifest_json = std::fs::read_to_string(&manifest_path).expect("lightpanda manifest");
    let runtime = BrowserRuntimeManager::production(&manifest_json).expect("runtime manager");
    let browser = Arc::new(
        BrowserManager::with_lightpanda(core.clone(), app_data_dir.clone(), &settings, runtime)
            .expect("browser manager"),
    );
    let capture: Arc<dyn ScreenCaptureProvider> = Arc::new(EmptyCapture);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mcp listener");
    let port = listener.local_addr().expect("mcp addr").port();
    let core_for_server = core.clone();
    let service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        move || {
            Ok(WorkspaceServer::new(
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
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mcp");
    });

    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://127.0.0.1:{port}/mcp"
        )))
        .await
        .expect("mcp client");

    RealEnv {
        client,
        driver,
        core,
        app_data_dir,
        _dir: dir,
    }
}

/// v6 settings removed the user-owned engine/run-mode choice: every real
/// session runs the fixed bundled backend in the background.
fn background_settings() -> AppSettings {
    AppSettings {
        browser_automation_enabled: true,
        ..AppSettings::default()
    }
}

// ---------------------------------------------------------------------------
// MCP call helpers
// ---------------------------------------------------------------------------

async fn call(client: &Client, tool: &str, args: Value) -> CallToolResult {
    let map = match args {
        Value::Object(m) => m,
        Value::Null => serde_json::Map::new(),
        other => panic!("tool args must be a JSON object, got {other}"),
    };
    client
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(map))
        .await
        .expect("mcp transport call failed")
}

fn is_ok(result: &CallToolResult) -> bool {
    !result.is_error.unwrap_or(false)
}

fn result_text(result: &CallToolResult) -> String {
    serde_json::to_string(result).unwrap_or_default()
}

fn structured(result: &CallToolResult) -> Value {
    result
        .structured_content
        .clone()
        .expect("structured content present")
}

fn assert_ok(result: &CallToolResult, context: &str) {
    assert!(
        is_ok(result),
        "{context} unexpectedly failed: {}",
        result_text(result)
    );
}

fn assert_error_code(result: &CallToolResult, code: ErrorCode, context: &str) {
    assert!(
        result.is_error.unwrap_or(false),
        "{context} unexpectedly succeeded: {}",
        result_text(result)
    );
    let text = result_text(result);
    assert!(
        text.contains(code.as_str()),
        "{context}: expected error code {} in: {text}",
        code.as_str()
    );
}

/// Extracts the `[ref=...]` token from the first snapshot line containing
/// `needle`.
fn find_ref(snapshot: &str, needle: &str) -> String {
    for line in snapshot.lines() {
        if line.contains(needle) {
            if let Some(start) = line.find("[ref=") {
                let rest = &line[start + "[ref=".len()..];
                if let Some(end) = rest.find(']') {
                    return rest[..end].to_string();
                }
            }
        }
    }
    panic!("ref for {needle:?} not found in snapshot:\n{snapshot}");
}

async fn take_snapshot(client: &Client) -> String {
    let result = call(client, "browser_snapshot", json!({})).await;
    assert_ok(&result, "browser_snapshot");
    structured(&result)["snapshot"]
        .as_str()
        .expect("snapshot text")
        .to_string()
}

/// Upstream `browser_evaluate` renders the return value as a markdown section:
/// `### Result\n<json>` (or an `### Error` section on failure). Strips any
/// leading header/blank lines and parses the remaining JSON value.
fn parse_evaluate_result(raw: &str) -> Value {
    let lines: Vec<&str> = raw.lines().collect();
    let mut start = 0;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            start = i + 1;
        } else {
            break;
        }
    }
    let body = lines[start..].join("\n");
    serde_json::from_str(body.trim())
        .unwrap_or_else(|e| panic!("cannot parse browser_evaluate result {body:?}: {e}"))
}

async fn evaluate(client: &Client, expression: &str) -> Value {
    let result = call(
        client,
        "browser_evaluate",
        json!({ "expression": expression }),
    )
    .await;
    assert_ok(&result, "browser_evaluate");
    let content = structured(&result);
    let raw = content["result"].as_str().expect("evaluate result text");
    parse_evaluate_result(raw)
}

/// Convenience for expressions that return a JS string.
async fn evaluate_str(client: &Client, expression: &str) -> String {
    let value = evaluate(client, expression).await;
    value
        .as_str()
        .unwrap_or_else(|| panic!("evaluate result is not a string: {value}"))
        .to_string()
}

/// Opens the browser and navigates to `url`; returns the structured payload.
async fn open_url(client: &Client, url: &str) -> Value {
    let result = call(client, "browser_open", json!({ "url": url })).await;
    assert_ok(&result, "browser_open");
    structured(&result)
}

async fn current_session_id(client: &Client) -> String {
    let result = call(client, "browser_get_status", json!({})).await;
    assert_ok(&result, "browser_get_status");
    structured(&result)["session_id"]
        .as_str()
        .expect("session_id")
        .to_string()
}

/// True when the network-request listing has a line containing both `path` and
/// `status` (e.g. `/api/ping` + `200`).
fn network_has(requests: &str, path: &str, status: &str) -> bool {
    requests
        .lines()
        .any(|line| line.contains(path) && line.contains(status))
}

/// Decodes the PNG image block from a screenshot result.
fn screenshot_png(result: &CallToolResult) -> Vec<u8> {
    let block = result
        .content
        .iter()
        .find_map(|b| b.as_image())
        .expect("screenshot result must contain an image block");
    assert_eq!(block.mime_type, "image/png", "screenshot must be PNG");
    base64::engine::general_purpose::STANDARD
        .decode(&block.data)
        .expect("valid base64 PNG")
}

/// The pinned playwright-mcp runtime (0.0.79) downscales every image it returns
/// over MCP via `scaleImageToFitMessage` so it fits an LLM message: max edge
/// 1568px and a 1.15 MiB pixel budget (`1.15 * 1024 * 1024` pixels), preserving
/// aspect ratio. Viewports under both limits pass through unchanged; larger ones
/// come back proportionally smaller. This replicates that exact rule so the test
/// can assert the precise capture size the runtime will return. The page-side
/// `window.innerWidth/innerHeight` probe separately proves the true viewport.
fn expected_mcp_capture_dims(width: u32, height: u32) -> (u32, u32) {
    let w = f64::from(width);
    let h = f64::from(height);
    let pixels = w * h;
    let shrink = (1568.0 / w)
        .min(1568.0 / h)
        .min((1.15 * 1024.0 * 1024.0 / pixels).sqrt());
    if shrink > 1.0 {
        return (width, height);
    }
    // `| 0` in the upstream JS truncates toward zero; `as u32` matches for +values.
    ((w * shrink) as u32, (h * shrink) as u32)
}

async fn wait_fixture_ready(fixture: &Fixture) {
    let http = reqwest::Client::new();
    let url = fixture.base_url();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if http
            .get(&url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("fixture server never became ready at {url}");
}

/// Reads the fixture's out-of-band action counter (number of executed Count
/// clicks) without going through the browser.
async fn fixture_count(fixture: &Fixture) -> usize {
    let url = format!("{}/api/count", fixture.base_url());
    let text = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .expect("count request")
        .text()
        .await
        .expect("count body");
    let body: Value = serde_json::from_str(&text).expect("count json");
    body["count"].as_u64().expect("count value") as usize
}

// ---------------------------------------------------------------------------
// Win32 helpers for the Background no-visible-window test
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win32 {
    use std::collections::HashSet;

    use windows::{
        core::BOOL,
        Win32::{
            Foundation::{CloseHandle, HWND, LPARAM, TRUE},
            System::Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
            UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible},
        },
    };

    /// Collects `root` plus every descendant pid by walking a system process
    /// snapshot (parent-pid chain).
    pub fn process_tree_pids(root: u32) -> HashSet<u32> {
        let mut pairs: Vec<(u32, u32)> = Vec::new(); // (pid, parent_pid)
        unsafe {
            if let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                let mut entry = PROCESSENTRY32W {
                    dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                    ..Default::default()
                };
                if Process32FirstW(snapshot, &mut entry).is_ok() {
                    loop {
                        pairs.push((entry.th32ProcessID, entry.th32ParentProcessID));
                        if Process32NextW(snapshot, &mut entry).is_err() {
                            break;
                        }
                    }
                }
                let _ = CloseHandle(snapshot);
            }
        }
        let mut tree: HashSet<u32> = HashSet::new();
        tree.insert(root);
        let mut changed = true;
        while changed {
            changed = false;
            for &(pid, ppid) in &pairs {
                if tree.contains(&ppid) && tree.insert(pid) {
                    changed = true;
                }
            }
        }
        tree
    }

    /// Pids that own at least one visible top-level window right now.
    pub fn visible_top_level_window_pids() -> HashSet<u32> {
        let mut pids: Vec<u32> = Vec::new();
        unsafe {
            let param = LPARAM(&mut pids as *mut Vec<u32> as isize);
            let _ = EnumWindows(Some(enum_windows_cb), param);
        }
        pids.into_iter().collect()
    }

    unsafe extern "system" fn enum_windows_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let pids = &mut *(lparam.0 as *mut Vec<u32>);
        if IsWindowVisible(hwnd).as_bool() {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid != 0 {
                pids.push(pid);
            }
        }
        TRUE
    }
}

// ---------------------------------------------------------------------------
// Step 2: real Background lifecycle / navigation / interaction
// (also satisfies the mandatory Bundled Chromium Background compatibility case)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_background_lifecycle_navigation_interaction() {
    let _guard = SERIAL.lock().await;
    if !runtime_available("background_lifecycle") {
        return;
    }

    let fixture = browser_test_app::spawn().await;
    wait_fixture_ready(&fixture).await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;
    let base = fixture.base_url();

    // Start Background BundledChromium + navigate to the fixture.
    let opened = open_url(client, &base).await;
    assert_eq!(opened["status"]["run_mode"], "background");
    assert_eq!(opened["status"]["actual_engine"], "bundled_chromium");
    assert!(opened["status"]["running"].as_bool().unwrap());

    // Snapshot -> resolve refs.
    let snap = take_snapshot(client).await;
    let name_ref = find_ref(&snap, "Name");
    let color_ref = find_ref(&snap, "Color");
    let submit_ref = find_ref(&snap, "Submit");

    // Fill / select / click.
    let fill = call(
        client,
        "browser_fill",
        json!({ "ref": name_ref, "text": "honey" }),
    )
    .await;
    assert_ok(&fill, "browser_fill");
    let select = call(
        client,
        "browser_select",
        json!({ "ref": color_ref, "values": ["green"] }),
    )
    .await;
    assert_ok(&select, "browser_select");
    let click = call(client, "browser_click", json!({ "ref": submit_ref })).await;
    assert_ok(&click, "browser_click");

    // Assert resulting DOM state.
    let result_text = evaluate_str(
        client,
        "() => document.getElementById('result').textContent",
    )
    .await;
    assert_eq!(result_text, "result:name=honey,color=green");

    // Assert known console messages (info level includes warning + error).
    let console = call(
        client,
        "browser_console",
        json!({ "level": "info", "limit": 200 }),
    )
    .await;
    assert_ok(&console, "browser_console");
    let messages = structured(&console)["messages"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        messages.contains("api-ping-complete"),
        "missing api-ping-complete in console:\n{messages}"
    );
    assert!(
        messages.contains("known-console-warning"),
        "missing known-console-warning in console:\n{messages}"
    );
    assert!(
        messages.contains("known-console-error"),
        "missing known-console-error in console:\n{messages}"
    );

    // Assert network metadata for the successful and failing API routes.
    let network = call(client, "browser_network", json!({ "limit": 200 })).await;
    assert_ok(&network, "browser_network");
    let requests = structured(&network)["requests"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        network_has(&requests, "/api/ping", "200"),
        "missing /api/ping 200 in network:\n{requests}"
    );
    assert!(
        network_has(&requests, "/api/fail", "500"),
        "missing /api/fail 500 in network:\n{requests}"
    );

    // browser_evaluate document.title.
    let title = evaluate_str(client, "() => document.title").await;
    assert_eq!(title, "Browser Test App");

    // Close cleanly.
    let close = call(client, "browser_close", json!({})).await;
    assert_ok(&close, "browser_close");
    let status = call(client, "browser_get_status", json!({})).await;
    assert!(!structured(&status)["running"].as_bool().unwrap());
}

// ---------------------------------------------------------------------------
// Step 3: responsive / screenshot / trace across four viewports
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_responsive_screenshot_trace() {
    let _guard = SERIAL.lock().await;
    if !runtime_available("responsive_screenshot_trace") {
        return;
    }

    let fixture = browser_test_app::spawn().await;
    wait_fixture_ready(&fixture).await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;
    open_url(client, &fixture.base_url()).await;

    let viewports: [(u32, u32, &str); 4] = [
        (320, 568, "mobile"),
        (375, 812, "mobile"),
        (768, 1024, "tablet"),
        (1440, 900, "desktop"),
    ];

    for (width, height, expected_mode) in viewports {
        // Local viewport logic (validates + browser_resize upstream).
        let set = call(
            client,
            "browser_set_viewport",
            json!({ "width": width, "height": height }),
        )
        .await;
        assert_ok(&set, "browser_set_viewport");

        // Prove window.innerWidth/innerHeight and responsive marker state. Poll
        // until the resize settles AND the page's resize handler has updated the
        // marker (the handler can lag the metric change by an event-loop tick).
        let deadline = Instant::now() + Duration::from_secs(10);
        let probe = loop {
            let probe = evaluate(
                client,
                "() => ({ w: window.innerWidth, h: window.innerHeight, mode: document.getElementById('responsive-marker').getAttribute('data-mode') })",
            )
            .await;
            if probe["w"].as_u64() == Some(width as u64)
                && probe["h"].as_u64() == Some(height as u64)
                && probe["mode"].as_str() == Some(expected_mode)
            {
                break probe;
            }
            assert!(
                Instant::now() < deadline,
                "viewport/marker did not settle to {width}x{height}/{expected_mode}; probe = {probe}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        assert_eq!(probe["mode"].as_str().unwrap(), expected_mode);

        // Capture PNG and validate decoded dimensions + hash. The pinned
        // playwright-mcp runtime downscales every image it returns over MCP to
        // fit a 1568px max edge / 1.15MP pixel budget (scaleImageToFitMessage),
        // so the expected capture size is the post-shrink size, while the page
        // metrics above prove the true viewport. Retry until the compositor
        // settles at the expected capture size.
        let (expected_w, expected_h) = expected_mcp_capture_dims(width, height);
        let (png, meta) = loop {
            let shot = call(client, "browser_screenshot", json!({ "mode": "viewport" })).await;
            assert_ok(&shot, "browser_screenshot");
            let meta = structured(&shot);
            let png = screenshot_png(&shot);
            let decoded =
                image::load_from_memory_with_format(&png, ImageFormat::Png).expect("decode PNG");
            let dims = decoded.dimensions();
            if dims == (expected_w, expected_h) {
                break (png, meta);
            }
            assert!(
                Instant::now() < deadline,
                "screenshot did not settle to {expected_w}x{expected_h} for viewport {width}x{height}; last capture was {}x{}",
                dims.0,
                dims.1
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        };
        assert_eq!(
            meta["original_width"].as_u64().unwrap(),
            expected_w as u64,
            "metadata original_width for {width}x{height}"
        );
        assert_eq!(
            meta["original_height"].as_u64().unwrap(),
            expected_h as u64,
            "metadata original_height for {width}x{height}"
        );
        assert_eq!(
            meta["sha256"].as_str().unwrap(),
            lc_core::ops::sha256_hex(&png),
            "screenshot sha256 must match returned PNG bytes"
        );
    }

    // Start/stop tracing and prove the artifact resolves inside the active
    // session trace directory.
    let trace_start = call(client, "browser_trace", json!({ "action": "start" })).await;
    assert_ok(&trace_start, "browser_trace start");
    assert_eq!(structured(&trace_start)["tracing"], true);

    // Generate some traced activity.
    let snap = take_snapshot(client).await;
    let submit_ref = find_ref(&snap, "Submit");
    let _ = call(client, "browser_click", json!({ "ref": submit_ref })).await;

    let trace_stop = call(client, "browser_trace", json!({ "action": "stop" })).await;
    assert_ok(&trace_stop, "browser_trace stop");
    let stop = structured(&trace_stop);
    assert_eq!(stop["tracing"], false);
    let artifact_id = stop["artifact_id"]
        .as_str()
        .expect("artifact_id")
        .to_string();
    assert!(
        artifact_id.starts_with("a-"),
        "opaque artifact id: {artifact_id}"
    );
    assert!(stop["files"].as_u64().unwrap() > 0, "trace files recorded");
    assert!(stop["bytes"].as_u64().unwrap() > 0, "trace bytes recorded");

    // The trace artifact must live inside the active session's trace directory.
    let session_id = current_session_id(client).await;
    let trace_dir = env
        .app_data_dir
        .join("browser")
        .join("artifacts")
        .join(&session_id)
        .join("traces");
    assert!(
        trace_dir.is_dir(),
        "trace dir must exist: {}",
        trace_dir.display()
    );
    let mut trace_files = 0usize;
    let mut stack = vec![trace_dir.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read trace dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                trace_files += 1;
            }
        }
    }
    assert!(
        trace_files > 0,
        "trace artifact files must resolve inside {}",
        trace_dir.display()
    );

    let close = call(client, "browser_close", json!({})).await;
    assert_ok(&close, "browser_close");
}

// ---------------------------------------------------------------------------
// Step 4: upload / download confinement end-to-end
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_upload_download_confinement() {
    let _guard = SERIAL.lock().await;
    if !runtime_available("upload_download_confinement") {
        return;
    }

    let fixture = browser_test_app::spawn().await;
    wait_fixture_ready(&fixture).await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;

    // Approved workspace with an upload source file.
    let workspace_root = env.app_data_dir.join("ws");
    std::fs::create_dir_all(&workspace_root).unwrap();
    std::fs::write(workspace_root.join("upload.txt"), b"upload-body").unwrap();
    let ws = lc_core::workspace_repo::insert_workspace(
        &env.core.db,
        "BrowserFiles",
        &workspace_root.to_string_lossy(),
    )
    .unwrap();
    lc_core::workspace_repo::update_capabilities(&env.core.db, &ws.id, true, true).unwrap();

    open_url(client, &fixture.base_url()).await;

    // Open the file chooser by clicking the upload input, then stage/upload
    // the workspace file through Rust.
    let snap = take_snapshot(client).await;
    let upload_ref = find_ref(&snap, "Upload");
    let _ = call(client, "browser_click", json!({ "ref": upload_ref })).await;

    let upload = call(
        client,
        "browser_upload",
        json!({
            "workspace_id": ws.id,
            "sources": [{ "kind": "workspace", "path": "upload.txt" }],
        }),
    )
    .await;
    assert_ok(&upload, "browser_upload");

    // The page reports the uploaded file name: an opaque staged id that keeps
    // only the sanitized extension, never the original workspace filename.
    let mut reported = String::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        reported = evaluate_str(
            client,
            "() => document.getElementById('upload-result').textContent",
        )
        .await;
        if reported.starts_with("upload:u-") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let name = reported.strip_prefix("upload:").unwrap_or(&reported);
    assert!(
        name.starts_with("u-") && name.ends_with(".txt"),
        "page must report an opaque staged upload name with preserved extension, got: {reported}"
    );
    assert!(
        !name.contains("upload.txt"),
        "staged upload must not leak the original filename: {name}"
    );

    // Trigger the deterministic download.
    let snap = take_snapshot(client).await;
    let download_ref = find_ref(&snap, "DownloadFile");
    let dl_click = call(client, "browser_click", json!({ "ref": download_ref })).await;
    assert_ok(&dl_click, "click download link");

    // Poll until the download is listed by opaque id.
    let mut download_id = String::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let list = call(client, "browser_downloads", json!({})).await;
        if is_ok(&list) {
            let arr = structured(&list);
            if let Some(items) = arr.as_array() {
                if let Some(item) = items.iter().find(|d| d["filename"] == "download.txt") {
                    if item["completed"].as_bool().unwrap_or(false) {
                        download_id = item["download_id"].as_str().unwrap().to_string();
                        break;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        !download_id.is_empty(),
        "download.txt was not listed as a completed download"
    );

    // Export to an approved workspace path and verify bytes/hash.
    let save = call(
        client,
        "browser_save_download_to_workspace",
        json!({
            "workspace_id": ws.id,
            "download_id": download_id,
            "path": "saved/download.txt",
        }),
    )
    .await;
    assert_ok(&save, "browser_save_download_to_workspace");
    let saved_bytes = std::fs::read(workspace_root.join("saved/download.txt")).unwrap();
    assert_eq!(saved_bytes, DOWNLOAD_BODY, "downloaded bytes must match");
    assert_eq!(
        structured(&save)["sha256"].as_str().unwrap(),
        lc_core::ops::sha256_hex(DOWNLOAD_BODY),
        "export sha256 must match deterministic body"
    );

    // A path-traversal export destination must be denied.
    let traversal = call(
        client,
        "browser_save_download_to_workspace",
        json!({
            "workspace_id": ws.id,
            "download_id": download_id,
            "path": "../escape.txt",
        }),
    )
    .await;
    assert_error_code(
        &traversal,
        ErrorCode::BrowserDownloadDenied,
        "traversal export",
    );
    assert!(
        !workspace_root.parent().unwrap().join("escape.txt").exists(),
        "traversal export must not write outside the workspace"
    );

    let close = call(client, "browser_close", json!({})).await;
    assert_ok(&close, "browser_close");
}

// ---------------------------------------------------------------------------
// Step 5: Background launch creates no visible top-level window (Windows)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(windows)]
async fn real_background_no_visible_window() {
    let _guard = SERIAL.lock().await;
    if !runtime_available("background_no_visible_window") {
        return;
    }

    let fixture = browser_test_app::spawn().await;
    wait_fixture_ready(&fixture).await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;
    open_url(client, &fixture.base_url()).await;
    // Give the browser tree a moment to spawn its child processes.
    tokio::time::sleep(Duration::from_millis(1000)).await;

    let root_pid = env
        .driver
        .test_child_pid()
        .await
        .expect("Playwright MCP child pid");
    let tree = win32::process_tree_pids(root_pid);
    assert!(
        tree.contains(&root_pid),
        "process tree must contain the root pid"
    );
    assert!(
        !tree.is_empty(),
        "process tree should include the browser child processes"
    );

    let visible = win32::visible_top_level_window_pids();
    let leaked: Vec<u32> = tree.intersection(&visible).copied().collect();
    assert!(
        leaked.is_empty(),
        "Background browser tree (root {root_pid}, {} processes) owns visible top-level windows: {leaked:?}",
        tree.len()
    );

    let close = call(client, "browser_close", json!({})).await;
    assert_ok(&close, "browser_close");
}

// ---------------------------------------------------------------------------
// Step 6a: crash during a read-only scenario -> one safe restart recovers
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_crash_readonly_safe_restart() {
    let _guard = SERIAL.lock().await;
    if !runtime_available("crash_readonly_safe_restart") {
        return;
    }

    let fixture = browser_test_app::spawn().await;
    wait_fixture_ready(&fixture).await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;
    open_url(client, &fixture.base_url()).await;
    let first_session = current_session_id(client).await;

    // Simulate an unexpected browser crash.
    let killed = env.driver.test_terminate_child().await.expect("kill child");
    println!("terminated Playwright MCP child pid {killed}");
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // A read-only snapshot against the lost process must surface
    // BROWSER_PROCESS_LOST (not hang, not silently succeed).
    let snap = call(client, "browser_snapshot", json!({})).await;
    assert_error_code(&snap, ErrorCode::BrowserProcessLost, "snapshot after crash");

    // One safe restart: close the dead session, then open a fresh one.
    let close = call(client, "browser_close", json!({})).await;
    assert_ok(&close, "browser_close after crash");
    let reopened = open_url(client, &fixture.base_url()).await;
    assert!(reopened["status"]["running"].as_bool().unwrap());
    let second_session = current_session_id(client).await;
    assert_ne!(
        first_session, second_session,
        "restart must establish a NEW session"
    );

    // The new session is fully usable.
    let snap = take_snapshot(client).await;
    assert!(snap.contains("Submit"), "new session snapshot works");
    let title = evaluate_str(client, "() => document.title").await;
    assert_eq!(title, "Browser Test App");

    let close = call(client, "browser_close", json!({})).await;
    assert_ok(&close, "final browser_close");
}

// ---------------------------------------------------------------------------
// Step 6b: crash before an ambiguous click -> BROWSER_PROCESS_LOST, zero replay
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_crash_ambiguous_click_no_replay() {
    let _guard = SERIAL.lock().await;
    if !runtime_available("crash_ambiguous_click_no_replay") {
        return;
    }

    let fixture = browser_test_app::spawn().await;
    wait_fixture_ready(&fixture).await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;
    open_url(client, &fixture.base_url()).await;

    // Execute one real side-effecting click; the fixture's out-of-band counter
    // proves it ran exactly once.
    let snap = take_snapshot(client).await;
    let count_ref = find_ref(&snap, "Count");
    let click = call(client, "browser_click", json!({ "ref": count_ref })).await;
    assert_ok(&click, "first count click");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && fixture_count(&fixture).await < 1 {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        fixture_count(&fixture).await,
        1,
        "first click executed once"
    );

    // Crash the browser, then attempt an ambiguous second click.
    let killed = env.driver.test_terminate_child().await.expect("kill child");
    println!("terminated Playwright MCP child pid {killed}");
    tokio::time::sleep(Duration::from_millis(1000)).await;

    let replayed = call(client, "browser_click", json!({ "ref": count_ref })).await;
    assert_error_code(
        &replayed,
        ErrorCode::BrowserProcessLost,
        "ambiguous click after crash",
    );

    // Zero automatic replay: the side-effecting click must not have been
    // re-executed against a restarted browser.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        fixture_count(&fixture).await,
        1,
        "side-effecting click must never be replayed after process loss"
    );

    let close = call(client, "browser_close", json!({})).await;
    assert_ok(&close, "browser_close");
}

// ---------------------------------------------------------------------------
// Step 7: engine selection removed (v6 settings) — the fixed bundled backend
// serves core navigation/interaction, and DevTools fails closed because
// Visible Debug is no longer a selectable run mode.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_fixed_backend_core_and_devtools_fail_closed() {
    let _guard = SERIAL.lock().await;
    if !runtime_available("fixed_backend_core") {
        return;
    }

    let fixture = browser_test_app::spawn().await;
    wait_fixture_ready(&fixture).await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;

    open_url(client, &fixture.base_url()).await;

    let snap = take_snapshot(client).await;
    let name_ref = find_ref(&snap, "Name");
    let submit_ref = find_ref(&snap, "Submit");
    let fill = call(
        client,
        "browser_fill",
        json!({ "ref": name_ref, "text": "fixed" }),
    )
    .await;
    assert_ok(&fill, "fixed backend browser_fill");
    let click = call(client, "browser_click", json!({ "ref": submit_ref })).await;
    assert_ok(&click, "fixed backend browser_click");
    let result_text = evaluate_str(
        client,
        "() => document.getElementById('result').textContent",
    )
    .await;
    assert_eq!(result_text, "result:name=fixed,color=red");

    // DevTools can never be supported now: Visible Debug is not a selectable
    // run mode under the fixed background backend.
    let devtools = call(client, "browser_devtools", json!({})).await;
    assert_error_code(
        &devtools,
        ErrorCode::BrowserFeatureUnsupportedInMode,
        "fixed backend browser_devtools",
    );

    let close = call(client, "browser_close", json!({})).await;
    assert_ok(&close, "browser_close");
}
