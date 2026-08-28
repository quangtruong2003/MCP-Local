//! Real Lightpanda-in-managed-WSL2 runtime integration tests (Task 14).
//!
//! Unlike `browser_mcp_integration.rs` (which drives a fake driver), this suite
//! drives the REAL production path end-to-end:
//!
//! ```text
//! MCP client -> WorkspaceServer handlers -> BrowserManager
//!   -> LightpandaPageDriver -> one `wsl.exe` child per page
//!   -> Lightpanda native MCP inside the app-owned WSL2 distro
//! ```
//!
//! The deterministic localhost fixture lives in `tests/fixtures/browser_test_app.rs`.
//!
//! # Skip gate
//!
//! Developer runs may skip these tests only when the prerequisites are
//! unavailable (non-Windows host, or WSL itself missing); every skip prints an
//! explicit reason. Setting `LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS=1` turns any
//! skip into a test failure, so release verification can never silently skip
//! the real runtime:
//!
//! ```text
//! LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS=1 \
//!   cargo test -p local-workspace-mcp-manager --test browser_runtime_integration --jobs 1 -- --nocapture
//! ```
//!
//! The first run provisions the real runtime (downloads the pinned rootfs and
//! Lightpanda 0.3.7, imports the app-owned distro) and can take several
//! minutes. Later runs reuse the persisted Ready state. Test names are
//! prefixed `t01_`..`t19_` so the serialized order is deterministic.

#[path = "fixtures/browser_test_app.rs"]
mod browser_test_app;

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{json, Value};

use lc_core::{error::ErrorCode, CoreServices};
use local_workspace_mcp_manager::{
    browser::{
        manager::BrowserManager,
        runtime::{
            BrowserRuntimeManager, HttpsRuntimeDownloader, RealRuntimeContractVerifier,
            RuntimeManifest, RuntimeProvisionState, RUNTIME_STATE_FILE,
        },
        types::BrowserRuntimeSettings,
        wsl::{decode_wsl_output, RealWslCommandRunner},
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

use browser_test_app::Fixture;

type Client = RunningService<RoleClient, ()>;

/// Serialize real-browser tests. Each launches actual WSL-backed page
/// processes against one shared managed distro; running them in parallel would
/// contend for the runtime and make process-tree assertions noisy. `--jobs 1`
/// serializes test *binaries*, not the threads inside one binary, so we
/// serialize explicitly here. The `tNN_` name prefixes make the lock order
/// deterministic.
static SERIAL: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

// ---------------------------------------------------------------------------
// Runtime gate: skip with a printed reason, fail closed under the release env
// ---------------------------------------------------------------------------

/// Returns true when the real WSL2 Lightpanda runtime can run on this host.
/// Otherwise prints a skip reason and returns false — unless
/// `LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS=1` is set, in which case any skip is a
/// hard test failure.
fn require_runtime(case: &str) -> bool {
    let required = std::env::var("LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS").as_deref() == Ok("1");
    let reason = if !cfg!(windows) {
        Some("the managed WSL2 Lightpanda runtime requires a Windows host".to_string())
    } else {
        match std::process::Command::new("wsl.exe")
            .arg("--status")
            .output()
        {
            Ok(output) if output.status.success() => None,
            Ok(output) => Some(format!(
                "wsl.exe --status exited with {} (WSL unavailable or not configured)",
                output.status
            )),
            Err(error) => Some(format!("wsl.exe could not be executed: {error}")),
        }
    };
    match reason {
        None => true,
        Some(reason) => {
            if required {
                panic!(
                    "FAIL [{case}]: real Lightpanda runtime test skipped while \
                     LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS=1: {reason}"
                );
            }
            println!("SKIP [{case}]: {reason}");
            false
        }
    }
}

fn manifest_json() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("lightpanda-runtime-manifest.json");
    std::fs::read_to_string(&path).expect("bundled lightpanda runtime manifest")
}

/// Production wiring against the real LocalAppData runtime root, exactly like
/// the desktop app.
fn production_runtime() -> Arc<BrowserRuntimeManager> {
    BrowserRuntimeManager::production(&manifest_json()).expect("runtime manager")
}

/// True when `wsl.exe --list` still registers a distro with this exact name.
fn wsl_distro_registered(name: &str) -> bool {
    let output = std::process::Command::new("wsl.exe")
        .args(["--list", "--quiet"])
        .output()
        .expect("wsl.exe --list --quiet");
    decode_wsl_output(&output.stdout)
        .lines()
        .any(|line| line.trim() == name)
}

/// Parses the default-route gateway (the Windows host IP as seen from inside
/// the WSL2 VM) from `/proc/net/route` content. The gateway column is 8 hex
/// chars in little-endian byte order.
fn parse_wsl_gateway_ip(route_table: &str) -> Option<String> {
    for line in route_table.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // Iface Destination Gateway ...; default route has destination 0.
        if fields.len() >= 3 && fields[1] == "00000000" && fields[2].len() == 8 {
            let bytes = (0..4)
                .map(|i| u8::from_str_radix(&fields[2][i * 2..i * 2 + 2], 16).ok())
                .collect::<Option<Vec<u8>>>()?;
            // Little-endian stored: least significant byte first.
            return Some(format!(
                "{}.{}.{}.{}",
                bytes[3], bytes[2], bytes[1], bytes[0]
            ));
        }
    }
    None
}

/// The host address the browser child must use to reach the Windows-side
/// fixture: the WSL2 NAT gateway IP of the app-owned distro. Falls back to
/// loopback when it cannot be determined (e.g. mirrored networking).
static WSL_FIXTURE_HOST: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let fallback = "127.0.0.1".to_string();
    let Some(distro) = production_runtime().distro_identity() else {
        return fallback;
    };
    let Ok(output) = std::process::Command::new("wsl.exe")
        .args(["-d", &distro.distro_name, "--", "cat", "/proc/net/route"])
        .output()
    else {
        return fallback;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    parse_wsl_gateway_ip(&text).unwrap_or(fallback)
});

/// Minimal node TCP proxy: listens on all interfaces (node.exe is a build
/// prerequisite of this repo and is allowed through the Windows Firewall,
/// unlike the freshly compiled test binary) and pipes every connection to
/// the Windows-side loopback fixture. Prints the bound port once listening.
const FIXTURE_PROXY_JS: &str = r#"
const net = require("net");
const fixPort = parseInt(process.argv[2], 10);
const server = net.createServer((sock) => {
  const up = net.connect(fixPort, "127.0.0.1");
  up.on("connect", () => { sock.pipe(up); up.pipe(sock); });
  up.on("error", () => sock.destroy());
  sock.on("error", () => up.destroy());
});
server.listen(0, "0.0.0.0", () => console.log("PROXY_PORT=" + server.address().port));
"#;

/// Deterministic fixture plus its WSL-side reachability bridge.
///
/// The browser child runs inside the WSL2 VM. It cannot reach the
/// Windows-side loopback fixture directly: WSL2 NAT has no VM->host
/// loopback forwarding, and Windows Firewall blocks inbound connections to
/// the freshly compiled test binary (the rule it creates is bound to one
/// binary hash and stops matching after every recompile). A tiny node TCP
/// proxy listening on the WSL-facing gateway IP bridges the two.
struct WslFixture {
    fixture: Fixture,
    /// Fixture URL reachable from inside the WSL2 VM (gateway host + proxy port).
    url: String,
    proxy_port: u16,
    proxy: tokio::process::Child,
    script_path: PathBuf,
}

impl Drop for WslFixture {
    fn drop(&mut self) {
        let _ = self.proxy.start_kill();
        let _ = std::fs::remove_file(&self.script_path);
    }
}

/// Spawns the fixture and its WSL-side proxy; the proxy must report its port
/// within 10 seconds.
async fn spawn_wsl_fixture() -> WslFixture {
    let fixture = browser_test_app::spawn().await;
    wait_fixture_ready(&fixture).await;

    let script_path = std::env::temp_dir().join(format!(
        "lwsl-fixture-proxy-{}-{}.cjs",
        std::process::id(),
        fixture.port
    ));
    std::fs::write(&script_path, FIXTURE_PROXY_JS).expect("write fixture proxy script");
    let mut child = tokio::process::Command::new("node")
        .arg(&script_path)
        .arg(fixture.port.to_string())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("node must be available to bridge the WSL fixture");

    use tokio::io::AsyncBufReadExt as _;
    let mut line = String::new();
    let mut reader = tokio::io::BufReader::new(child.stdout.as_mut().expect("proxy stdout"));
    tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line))
        .await
        .expect("fixture proxy must print its port within 10s")
        .expect("read fixture proxy port line");
    let proxy_port: u16 = line
        .trim()
        .strip_prefix("PROXY_PORT=")
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("unexpected fixture proxy output: {line:?}"));
    let url = format!("http://{}:{proxy_port}", &*WSL_FIXTURE_HOST);
    WslFixture {
        fixture,
        url,
        proxy_port,
        proxy: child,
        script_path,
    }
}

// ---------------------------------------------------------------------------
// Process liveness helpers (Windows host)
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code: u32 = 0;
        let alive = GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE;
        let _ = CloseHandle(handle);
        alive
    }
}

#[cfg(not(windows))]
fn process_alive(_pid: u32) -> bool {
    unreachable!("real runtime tests require a Windows host and are gated above")
}

async fn wait_process_dead(pid: u32, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if !process_alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{what} (pid {pid}) is still alive after termination");
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
    browser: Arc<BrowserManager>,
    runtime: Arc<BrowserRuntimeManager>,
    core: Arc<CoreServices>,
    app_data_dir: PathBuf,
    _dir: tempfile::TempDir,
}

/// Spawns the full production MCP HTTP server wired exactly like the desktop
/// app: production runtime manager (real LocalAppData root) plus the
/// Lightpanda page-child manager. Nothing is provisioned until the first page
/// operation passes the runtime gate.
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
    let runtime = production_runtime();
    let browser = Arc::new(
        BrowserManager::with_lightpanda(
            core.clone(),
            app_data_dir.clone(),
            &settings,
            runtime.clone(),
        )
        .expect("browser manager"),
    );
    let capture: Arc<dyn ScreenCaptureProvider> = Arc::new(EmptyCapture);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mcp listener");
    let port = listener.local_addr().expect("mcp addr").port();
    let core_for_server = core.clone();
    let browser_for_server = browser.clone();
    let service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        move || {
            Ok(WorkspaceServer::new(
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
        browser,
        runtime,
        core,
        app_data_dir,
        _dir: dir,
    }
}

/// v6 settings removed the user-owned engine/run-mode choice: every real
/// session runs the fixed Lightpanda backend in the background.
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

/// Extracts the leading `eN` ref token from the first snapshot line
/// containing `needle`. Interactive lines (marked `[i]`) win over plain text
/// nodes so action helpers target the control, not its label.
fn find_ref(snapshot: &str, needle: &str) -> String {
    let mut fallback: Option<String> = None;
    for line in snapshot.lines() {
        if !line.contains(needle) {
            continue;
        }
        let token = line.split_whitespace().next().unwrap_or_default();
        let is_ref = token.len() > 1
            && token.starts_with('e')
            && token[1..].chars().all(|c| c.is_ascii_digit());
        if !is_ref {
            continue;
        }
        if line.contains("[i]") {
            return token.to_string();
        }
        if fallback.is_none() {
            fallback = Some(token.to_string());
        }
    }
    match fallback {
        Some(found) => found,
        None => panic!("ref for {needle:?} not found in snapshot:\n{snapshot}"),
    }
}

async fn take_snapshot(client: &Client) -> String {
    let result = call(client, "browser_snapshot", json!({})).await;
    assert_ok(&result, "browser_snapshot");
    structured(&result)["snapshot"]
        .as_str()
        .expect("snapshot text")
        .to_string()
}

/// Lightpanda's native `evaluate` tool renders the return value as plain
/// text: objects and arrays as JSON, `null`/booleans/numbers as their bare
/// literals, strings unquoted, and `undefined` as the literal word. Parse
/// JSON when the text is JSON; map `undefined` to null; otherwise keep the
/// raw string. (Bare numbers are ambiguous with numeric strings; tests that
/// need string fidelity wrap the expression in `JSON.stringify`.)
fn parse_evaluate_result(raw: &str) -> Value {
    let body = raw.trim();
    if body == "undefined" {
        return Value::Null;
    }
    serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.to_string()))
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

async fn close_browser(client: &Client, context: &str) {
    let result = call(client, "browser_close", json!({})).await;
    assert_ok(&result, context);
}

async fn current_session_id(client: &Client) -> String {
    let result = call(client, "browser_get_status", json!({})).await;
    assert_ok(&result, "browser_get_status");
    structured(&result)["session_id"]
        .as_str()
        .expect("session_id")
        .to_string()
}

/// Returns the `browser_tabs` list as JSON array of page summaries.
async fn tabs_list(client: &Client) -> Vec<Value> {
    let result = call(client, "browser_tabs", json!({ "action": "list" })).await;
    assert_ok(&result, "browser_tabs list");
    structured(&result)
        .as_array()
        .expect("tabs list array")
        .clone()
}

/// Creates a new page navigated to `url` and returns its page_id.
async fn tabs_new(client: &Client, url: &str) -> String {
    let result = call(
        client,
        "browser_tabs",
        json!({ "action": "new", "url": url }),
    )
    .await;
    assert_ok(&result, "browser_tabs new");
    let pages = structured(&result);
    let active = pages
        .as_array()
        .expect("tabs new array")
        .iter()
        .find(|p| p["active"].as_bool().unwrap_or(false))
        .expect("new page is active")
        .clone();
    active["page_id"].as_str().expect("page_id").to_string()
}

async fn tabs_select(client: &Client, page_id: &str) {
    let result = call(
        client,
        "browser_tabs",
        json!({ "action": "select", "page_id": page_id }),
    )
    .await;
    assert_ok(&result, "browser_tabs select");
}

async fn tabs_close(client: &Client, page_id: &str) {
    let result = call(
        client,
        "browser_tabs",
        json!({ "action": "close", "page_id": page_id }),
    )
    .await;
    assert_ok(&result, "browser_tabs close");
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
// Win32 helpers for the no-visible-window test
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
// t01: already-provisioned runtime -> ready
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t01_already_provisioned_runtime_is_ready() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("already_provisioned_runtime_is_ready") {
        return;
    }

    // First manager provisions (or re-verifies) the pinned runtime. On a
    // fresh machine this downloads the rootfs + Lightpanda and imports the
    // app-owned distro; later runs re-verify the persisted state.
    let runtime = production_runtime();
    let state = runtime
        .ensure_runtime()
        .await
        .expect("runtime provisions to Ready");
    assert_eq!(state, RuntimeProvisionState::Ready);

    // A brand-new manager instance must load Ready from the persisted state
    // file without re-provisioning: the host is already prepared.
    let fresh = production_runtime();
    assert_eq!(fresh.current_state(), RuntimeProvisionState::Ready);
    let snapshot = fresh.state_snapshot().expect("persisted state snapshot");
    assert_eq!(snapshot.state, RuntimeProvisionState::Ready);
    assert!(
        snapshot.install_id.len() == 8
            && snapshot
                .install_id
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "install id must be 8 lowercase hex chars: {}",
        snapshot.install_id
    );
    let distro = snapshot.distro.expect("managed distro identity");
    assert!(
        distro.distro_name.starts_with("LocalWorkspaces-Lightpanda"),
        "collision-safe distro name: {}",
        distro.distro_name
    );
    assert_eq!(distro.install_id, snapshot.install_id);
    assert_eq!(
        snapshot.lightpanda_version.as_deref(),
        Some("0.3.7"),
        "Lightpanda must stay pinned to 0.3.7"
    );
    assert!(
        wsl_distro_registered(&distro.distro_name),
        "managed distro must be registered with WSL: {}",
        distro.distro_name
    );
}

// ---------------------------------------------------------------------------
// t02: background launch with no visible browser UI
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t02_background_launch_no_visible_window() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("background_launch_no_visible_window") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;

    let opened = open_url(client, &wf.url).await;
    assert_eq!(opened["status"]["run_mode"], "background");
    assert!(opened["status"]["running"].as_bool().unwrap());

    // Give the page child a moment to settle its process tree.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let pid = env
        .browser
        .test_page_child_pid(None)
        .await
        .expect("page child pid");

    #[cfg(windows)]
    {
        let tree = win32::process_tree_pids(pid);
        assert!(tree.contains(&pid), "tree must contain its root pid");
        let visible = win32::visible_top_level_window_pids();
        let leaked: Vec<u32> = tree.intersection(&visible).copied().collect();
        assert!(
            leaked.is_empty(),
            "background browser tree (root {pid}, {} processes) owns visible top-level windows: {leaked:?}",
            tree.len()
        );
    }

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t03: navigation + verified final URL
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t03_navigation_verifies_final_url() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("navigation_verifies_final_url") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;
    let base = wf.url.clone();

    open_url(client, &base).await;
    let target = format!("{base}/download.txt");

    // goto success alone is insufficient: the handler validates the URL via a
    // follow-up getUrl, and the page summary must agree.
    let nav = call(
        client,
        "browser_navigate",
        json!({ "action": "url", "url": target }),
    )
    .await;
    assert_ok(&nav, "browser_navigate url");
    let nav_out = structured(&nav);
    assert_eq!(
        nav_out["text"].as_str().unwrap(),
        target,
        "verified final URL"
    );
    let active = nav_out["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["active"].as_bool().unwrap_or(false))
        .expect("active page summary")
        .clone();
    assert_eq!(active["url"].as_str().unwrap(), target);

    // Reload keeps the verified URL.
    let reload = call(client, "browser_navigate", json!({ "action": "reload" })).await;
    assert_ok(&reload, "browser_navigate reload");
    assert_eq!(structured(&reload)["text"].as_str().unwrap(), target);

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t04: tree -> opaque ref -> selector-resolved fill/select/click
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t04_tree_ref_selector_fill_select_click() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("tree_ref_selector_fill_select_click") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;

    open_url(client, &wf.url).await;

    // Each side-effecting action invalidates the current ref generation, so a
    // fresh snapshot is taken before every action (the intended model usage).
    let snap = take_snapshot(client).await;
    let name_ref = find_ref(&snap, "Name");
    let fill = call(
        client,
        "browser_fill",
        json!({ "ref": name_ref, "text": "honey" }),
    )
    .await;
    assert_ok(&fill, "browser_fill");

    let snap = take_snapshot(client).await;
    let color_ref = find_ref(&snap, "Color");
    let select = call(
        client,
        "browser_select",
        json!({ "ref": color_ref, "values": ["green"] }),
    )
    .await;
    assert_ok(&select, "browser_select");

    let snap = take_snapshot(client).await;
    let submit_ref = find_ref(&snap, "Submit");
    let click = call(client, "browser_click", json!({ "ref": submit_ref })).await;
    assert_ok(&click, "browser_click");

    let result_text = evaluate_str(
        client,
        "() => document.getElementById('result').textContent",
    )
    .await;
    assert_eq!(result_text, "result:name=honey,color=green");

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t05: all ten native-first wrapper tools
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t05_all_ten_native_first_wrappers() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("all_ten_native_first_wrappers") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;

    open_url(client, &wf.url).await;

    // 1. browser_markdown: whole page render.
    let md = call(client, "browser_markdown", json!({})).await;
    assert_ok(&md, "browser_markdown");
    let md_text = structured(&md)["text"].as_str().unwrap().to_string();
    assert!(
        md_text.contains("Browser Test App"),
        "markdown must contain the page heading:\n{md_text}"
    );

    // 2. browser_html: whole document dump.
    let html = call(client, "browser_html", json!({})).await;
    assert_ok(&html, "browser_html");
    let html_text = structured(&html)["text"].as_str().unwrap().to_string();
    assert!(
        html_text.contains("name-input"),
        "html must contain the page markup"
    );

    // 3. browser_html scoped to one snapshot ref. Targets the Name textbox
    // (an element with a stable id/selector); a text-only node has no usable
    // selector under the pinned runtime. The result must be that element's
    // outerHTML, not the whole document.
    let snap = take_snapshot(client).await;
    let name_ref = find_ref(&snap, "Name");
    let scoped = call(client, "browser_html", json!({ "ref": name_ref })).await;
    assert_ok(&scoped, "browser_html scoped");
    let scoped_text = structured(&scoped)["text"].as_str().unwrap().to_string();
    assert!(
        scoped_text.contains("name-input"),
        "scoped html must be the referenced element's outerHTML:\n{scoped_text}"
    );
    assert!(
        !scoped_text.contains("DownloadFile"),
        "scoped html must not be the whole document:\n{scoped_text}"
    );

    // 4. browser_links.
    let links = call(client, "browser_links", json!({})).await;
    assert_ok(&links, "browser_links");
    let links_text = structured(&links)["text"].as_str().unwrap().to_string();
    assert!(
        links_text.contains("/download.txt"),
        "links must include the download link:\n{links_text}"
    );

    // 5. browser_find_element by role + accessible name.
    let found = call(
        client,
        "browser_find_element",
        json!({ "role": "button", "name": "Submit" }),
    )
    .await;
    assert_ok(&found, "browser_find_element");
    let found_text = structured(&found)["text"].as_str().unwrap().to_string();
    assert!(
        found_text.contains("Submit"),
        "find_element must match the Submit button:\n{found_text}"
    );

    // 6. browser_interactive_elements.
    let interactive = call(client, "browser_interactive_elements", json!({})).await;
    assert_ok(&interactive, "browser_interactive_elements");
    let interactive_text = structured(&interactive)["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        interactive_text.contains("Submit"),
        "interactive elements must list the Submit button:\n{interactive_text}"
    );

    // 7. browser_extract with a validated CSS-selector schema.
    let extract = call(
        client,
        "browser_extract",
        json!({ "schema": { "title": "title", "result": "#result" } }),
    )
    .await;
    assert_ok(&extract, "browser_extract");
    let data = structured(&extract)["data"].clone();
    assert!(data.is_object(), "extract data must be an object: {data}");
    assert!(
        data.to_string().contains("Browser Test App"),
        "extract must pull the title text: {data}"
    );

    // 8. browser_structured_data (fixture has no JSON-LD; shape must hold).
    let structured_data = call(client, "browser_structured_data", json!({})).await;
    assert_ok(&structured_data, "browser_structured_data");
    assert!(structured(&structured_data)["text"].is_string());

    // 9. browser_detect_forms (values are never included).
    let forms = call(client, "browser_detect_forms", json!({})).await;
    assert_ok(&forms, "browser_detect_forms");
    let forms_text = structured(&forms)["text"].as_str().unwrap().to_string();
    assert!(
        !forms_text.contains("honey"),
        "detect_forms must never include filled values:\n{forms_text}"
    );

    // 10a. browser_scroll on the window.
    let scroll = call(client, "browser_scroll", json!({ "y": 120 })).await;
    assert_ok(&scroll, "browser_scroll");

    // 10b. browser_set_checked on the checkbox ref (check + uncheck).
    let snap = take_snapshot(client).await;
    let checkbox_ref = find_ref(&snap, "Agree");
    let check = call(
        client,
        "browser_set_checked",
        json!({ "ref": checkbox_ref, "checked": true }),
    )
    .await;
    assert_ok(&check, "browser_set_checked true");
    assert_eq!(
        evaluate(
            client,
            "() => document.getElementById('agree-checkbox').checked"
        )
        .await,
        json!(true)
    );
    let snap = take_snapshot(client).await;
    let checkbox_ref = find_ref(&snap, "Agree");
    let uncheck = call(
        client,
        "browser_set_checked",
        json!({ "ref": checkbox_ref, "checked": false }),
    )
    .await;
    assert_ok(&uncheck, "browser_set_checked false");
    assert_eq!(
        evaluate(
            client,
            "() => document.getElementById('agree-checkbox').checked"
        )
        .await,
        json!(false)
    );

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t06: page-context evaluate
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t06_page_context_evaluate() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("page_context_evaluate") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;

    open_url(client, &wf.url).await;

    assert_eq!(
        evaluate_str(client, "() => document.title").await,
        "Browser Test App"
    );
    assert_eq!(evaluate(client, "6 * 7").await, json!(42));
    assert_eq!(
        evaluate(client, "() => ({ ok: true, n: 3 })").await,
        json!({ "ok": true, "n": 3 })
    );
    assert_eq!(evaluate(client, "() => [1, 2, 3]").await, json!([1, 2, 3]));

    // Ref-scoped evaluate: the expression runs against the resolved element.
    let snap = take_snapshot(client).await;
    let name_ref = find_ref(&snap, "Name");
    let scoped = call(
        client,
        "browser_evaluate",
        json!({ "expression": "el.tagName", "ref": name_ref }),
    )
    .await;
    assert_ok(&scoped, "ref-scoped browser_evaluate");
    let scoped_out = structured(&scoped);
    let raw = scoped_out["result"].as_str().expect("result text");
    assert_eq!(parse_evaluate_result(raw), json!("INPUT"));

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t07: every wait condition and load-state value
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t07_every_wait_condition_and_load_state() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("every_wait_condition_and_load_state") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;

    open_url(client, &wf.url).await;

    for condition in [
        json!({ "kind": "text_present", "text": "Browser Test App" }),
        json!({ "kind": "text_absent", "text": "no-such-text-anywhere" }),
        json!({ "kind": "url_contains", "value": format!(":{}", wf.proxy_port) }),
        json!({ "kind": "load_state", "state": "complete" }),
        json!({ "kind": "delay", "milliseconds": 200 }),
    ] {
        let wait = call(
            client,
            "browser_wait",
            json!({ "condition": condition, "timeout_ms": 10000 }),
        )
        .await;
        assert_ok(&wait, &format!("browser_wait {condition}"));
    }

    // RefVisible on a live ref.
    let snap = take_snapshot(client).await;
    let submit_ref = find_ref(&snap, "Submit");
    let wait = call(
        client,
        "browser_wait",
        json!({ "condition": { "kind": "ref_visible", "ref_id": submit_ref }, "timeout_ms": 10000 }),
    )
    .await;
    assert_ok(&wait, "browser_wait ref_visible");

    // `loading` and `interactive` can never hold on a fully loaded page; the
    // conditions must time out deterministically with BROWSER_ACTION_TIMEOUT
    // (proving they execute their real scripts rather than passing trivially).
    for state in ["loading", "interactive"] {
        let wait = call(
            client,
            "browser_wait",
            json!({ "condition": { "kind": "load_state", "state": state }, "timeout_ms": 500 }),
        )
        .await;
        assert_error_code(
            &wait,
            ErrorCode::BrowserActionTimeout,
            &format!("load_state {state} on a loaded page"),
        );
    }

    // RefHidden: after a reload the old generation is stale, and hidden holds
    // immediately (documented fail-closed behavior).
    let reload = call(client, "browser_navigate", json!({ "action": "reload" })).await;
    assert_ok(&reload, "reload before ref_hidden");
    let wait = call(
        client,
        "browser_wait",
        json!({ "condition": { "kind": "ref_hidden", "ref_id": submit_ref }, "timeout_ms": 10000 }),
    )
    .await;
    assert_ok(&wait, "browser_wait ref_hidden after reload");

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t08: console ring with level filtering
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t08_console_ring_levels() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("console_ring_levels") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;

    open_url(client, &wf.url).await;

    // Poll until the page's /api/ping fetch has resolved and logged.
    let mut messages;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let console = call(
            client,
            "browser_console",
            json!({ "level": "info", "limit": 200 }),
        )
        .await;
        assert_ok(&console, "browser_console info");
        messages = structured(&console)["messages"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if messages.contains("api-ping-complete") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "api-ping-complete never appeared in console:\n{messages}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        messages.contains("known-console-warning"),
        "missing known-console-warning:\n{messages}"
    );
    assert!(
        messages.contains("known-console-error"),
        "missing known-console-error:\n{messages}"
    );

    // Level filter: error shows only errors.
    let console = call(client, "browser_console", json!({ "level": "error" })).await;
    assert_ok(&console, "browser_console error");
    let out = structured(&console);
    let errors = out["messages"].as_str().unwrap_or_default().to_string();
    assert!(
        errors.contains("known-console-error"),
        "error level must include the error line:\n{errors}"
    );
    assert!(
        !errors.contains("known-console-warning") && !errors.contains("api-ping-complete"),
        "error level must exclude warning/info lines:\n{errors}"
    );
    let levels = out["levels"].as_object().expect("levels histogram");
    assert!(
        levels.get("error").and_then(Value::as_u64).unwrap_or(0) >= 1,
        "levels histogram must count the error line: {levels:?}"
    );

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t09: degraded network/performance shape (never fabricated)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t09_degraded_network_performance_shape() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("degraded_network_performance_shape") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;

    open_url(client, &wf.url).await;

    // First prove the page's /api/ping fetch really happened by waiting for
    // its completion log in the console ring.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut messages;
    loop {
        let console = call(
            client,
            "browser_console",
            json!({ "level": "info", "limit": 200 }),
        )
        .await;
        assert_ok(&console, "browser_console info");
        messages = structured(&console)["messages"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if messages.contains("api-ping-complete") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "api-ping-complete never appeared in console:\n{messages}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // The pinned Lightpanda 0.3.7 records no Performance Resource Timing
    // entries (verified against the pinned binary for fetch, img, script and
    // link resources), so the truthful network output is empty — even though
    // the fetch above demonstrably completed. The adapter must report the
    // degraded coverage note and never fabricate requests or status fields.
    let network = call(client, "browser_network", json!({ "limit": 200 })).await;
    assert_ok(&network, "browser_network");
    let out = structured(&network);
    let requests = out["requests"].as_str().unwrap_or_default().to_string();
    assert!(
        requests.is_empty(),
        "pinned runtime records no resource timing; nothing may be fabricated:\n{requests}"
    );
    assert_eq!(
        out["count"].as_u64(),
        Some(0),
        "count must match the empty request list"
    );
    let coverage = out["coverage"].as_str().unwrap_or_default().to_string();
    assert!(
        coverage.contains("performance-resource-timing"),
        "coverage must name the real data source: {coverage}"
    );
    assert!(
        coverage.contains("method, status, and headers are not available"),
        "coverage must state what is unavailable: {coverage}"
    );
    assert!(
        !requests.contains("status="),
        "timing lines must not fabricate status fields:\n{requests}"
    );

    // Performance: Lightpanda navigation timing, CDP honestly unsupported.
    let perf = call(client, "browser_performance", json!({})).await;
    assert_ok(&perf, "browser_performance");
    let out = structured(&perf);
    assert_eq!(out["backend"].as_str(), Some("lightpanda"));
    assert_eq!(out["cdp_supported"], json!(false));
    assert!(out["cdp"].is_null(), "cdp must be null, never fabricated");
    assert!(
        out["page"].as_str().map(str::len).unwrap_or(0) > 0,
        "page timing section must be present"
    );

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t10: local trace artifact on disk
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t10_local_trace_artifact_on_disk() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("local_trace_artifact_on_disk") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;

    open_url(client, &wf.url).await;

    let start = call(client, "browser_trace", json!({ "action": "start" })).await;
    assert_ok(&start, "browser_trace start");
    assert_eq!(structured(&start)["tracing"], json!(true));

    // Generate traced activity.
    let snap = take_snapshot(client).await;
    let submit_ref = find_ref(&snap, "Submit");
    let click = call(client, "browser_click", json!({ "ref": submit_ref })).await;
    assert_ok(&click, "traced click");

    let stop = call(client, "browser_trace", json!({ "action": "stop" })).await;
    assert_ok(&stop, "browser_trace stop");
    let out = structured(&stop);
    assert_eq!(out["tracing"], json!(false));
    let artifact_id = out["artifact_id"]
        .as_str()
        .expect("artifact_id")
        .to_string();
    assert!(
        artifact_id.starts_with("a-"),
        "opaque artifact id: {artifact_id}"
    );
    assert_eq!(out["files"], json!(2), "events.jsonl + manifest.json");
    assert!(out["bytes"].as_u64().unwrap() > 0, "trace bytes recorded");

    // The artifact must resolve inside the active session's trace directory.
    let session_id = current_session_id(client).await;
    let trace_dir = env
        .app_data_dir
        .join("browser")
        .join("artifacts")
        .join(&session_id)
        .join("traces");
    let events_path = trace_dir.join("events.jsonl");
    let manifest_path = trace_dir.join("manifest.json");
    assert!(
        events_path.is_file(),
        "events.jsonl must exist: {}",
        events_path.display()
    );
    assert!(
        manifest_path.is_file(),
        "manifest.json must exist: {}",
        manifest_path.display()
    );
    let events_body = std::fs::read_to_string(&events_path).expect("read events.jsonl");
    let event_lines: Vec<&str> = events_body.lines().filter(|l| !l.is_empty()).collect();
    assert!(!event_lines.is_empty(), "trace events recorded");
    assert!(
        event_lines.iter().any(|line| line.contains("\"click\"")),
        "trace must include the click event:\n{events_body}"
    );
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("read manifest"))
            .expect("manifest json");
    assert_eq!(manifest["schema"], json!("lc-trace-v1"));
    assert_eq!(
        manifest["event_count"].as_u64().unwrap(),
        event_lines.len() as u64
    );

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t11: two page children and isolated crash
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t11_two_page_children_isolated_crash() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("two_page_children_isolated_crash") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;
    let base = wf.url.clone();

    open_url(client, &base).await;
    let pages = tabs_list(client).await;
    assert_eq!(pages.len(), 1);
    let first_page = pages[0]["page_id"].as_str().unwrap().to_string();

    let second_page = tabs_new(client, &base).await;
    assert_ne!(first_page, second_page);

    // One Lightpanda child per logical page: distinct OS pids.
    let pid_first = env
        .browser
        .test_page_child_pid(Some(&first_page))
        .await
        .expect("first page child pid");
    let pid_second = env
        .browser
        .test_page_child_pid(Some(&second_page))
        .await
        .expect("second page child pid");
    assert_ne!(
        pid_first, pid_second,
        "each page must own a separate child process"
    );

    // Crash only the second page.
    let killed = env
        .browser
        .test_terminate_page_child(Some(&second_page))
        .await
        .expect("kill second page child");
    println!("terminated second page child pid {killed}");
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // The lost page surfaces BROWSER_PROCESS_LOST...
    tabs_select(client, &second_page).await;
    let snap = call(client, "browser_snapshot", json!({})).await;
    assert_error_code(
        &snap,
        ErrorCode::BrowserProcessLost,
        "snapshot on lost page",
    );

    // ...while the sibling page keeps working.
    tabs_select(client, &first_page).await;
    let snap = take_snapshot(client).await;
    assert!(snap.contains("Submit"), "sibling page snapshot works");

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t12: multi-page Web Storage: per-page, isolated, volatile
//
// The pinned Lightpanda 0.3.7 `mcp` mode rejects `--storage-engine` and
// `--storage-sqlite-path` (verified against the pinned binary: "unknown
// argument"), so Web Storage cannot be shared through SQLite. This scenario
// pins the truthful behavior instead: storage works per page, is isolated
// between page children, and does not survive a session restart.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t12_multipage_storage_isolated_and_volatile() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("multipage_storage_isolated_and_volatile") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;
    let base = wf.url.clone();

    open_url(client, &base).await;
    let pages = tabs_list(client).await;
    let page_a = pages[0]["page_id"].as_str().unwrap().to_string();

    // Storage works on page A. Reads are wrapped in JSON.stringify so string
    // values keep their type against Lightpanda's bare-text evaluate output.
    evaluate(client, "localStorage.clear()").await;
    evaluate(client, "localStorage.setItem('k_a', 'A')").await;
    assert_eq!(
        evaluate(client, "JSON.stringify(localStorage.getItem('k_a'))").await,
        json!("A")
    );

    // A brand-new page child starts with its own storage: it does not inherit
    // page A's keys.
    let page_b = tabs_new(client, &base).await;
    assert!(
        evaluate(client, "localStorage.getItem('k_a')")
            .await
            .is_null(),
        "page B storage must be isolated from page A"
    );
    evaluate(client, "localStorage.setItem('k_b', 'B')").await;
    assert_eq!(
        evaluate(client, "JSON.stringify(localStorage.getItem('k_b'))").await,
        json!("B")
    );

    // Interleaved burst writes from BOTH page children. Public MCP routes
    // evaluate through the active page, so the writes interleave by selecting
    // pages; every write must succeed and stay in its own page's storage.
    for i in 0..5 {
        tabs_select(client, &page_a).await;
        let w = evaluate(
            client,
            &format!("localStorage.setItem('k_a_{i}', '{i}'); 'ok'"),
        )
        .await;
        assert_eq!(w, json!("ok"), "page A burst write {i}");
        tabs_select(client, &page_b).await;
        let w = evaluate(
            client,
            &format!("localStorage.setItem('k_b_{i}', '{i}'); 'ok'"),
        )
        .await;
        assert_eq!(w, json!("ok"), "page B burst write {i}");
    }

    // Each page reads back its own writes and never sees the sibling's.
    tabs_select(client, &page_a).await;
    assert_eq!(
        evaluate(client, "JSON.stringify(localStorage.getItem('k_a_4'))").await,
        json!("4")
    );
    assert!(
        evaluate(client, "localStorage.getItem('k_b_4')")
            .await
            .is_null(),
        "page A must not see page B's keys"
    );
    tabs_select(client, &page_b).await;
    assert_eq!(
        evaluate(client, "JSON.stringify(localStorage.getItem('k_b_4'))").await,
        json!("4")
    );
    assert!(
        evaluate(client, "localStorage.getItem('k_a_4')")
            .await
            .is_null(),
        "page B must not see page A's keys"
    );

    // Volatile: after a session restart, storage starts empty.
    close_browser(client, "browser_close before reopen").await;
    open_url(client, &base).await;
    assert!(
        evaluate(client, "localStorage.getItem('k_a')")
            .await
            .is_null(),
        "storage must not persist across sessions on the pinned runtime"
    );
    assert!(
        evaluate(client, "localStorage.getItem('k_b_4')")
            .await
            .is_null(),
        "burst writes must not persist across sessions on the pinned runtime"
    );

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t13: cookie persistence and documented non-live sibling behavior
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t13_cookie_persistence_non_live_sibling() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("cookie_persistence_non_live_sibling") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;
    let base = wf.url.clone();
    let cookie = format!("live13_{}", std::process::id());

    open_url(client, &base).await;
    let pages = tabs_list(client).await;
    let page_a = pages[0]["page_id"].as_str().unwrap().to_string();

    // Page B is seeded from the canonical snapshot BEFORE the cookie exists.
    let page_b = tabs_new(client, &base).await;

    // Page A sets the cookie; it sees it immediately.
    tabs_select(client, &page_a).await;
    evaluate(
        client,
        &format!("document.cookie = '{cookie}=1; path=/'; 'set'"),
    )
    .await;
    let visible = evaluate_str(client, "() => document.cookie").await;
    assert!(
        visible.contains(&cookie),
        "page A must see its own cookie: {visible}"
    );

    // Documented non-live sibling behavior: the already-running page B keeps
    // its spawn-time jar and does NOT see cookies set later by a sibling.
    tabs_select(client, &page_b).await;
    let sibling = evaluate_str(client, "() => document.cookie").await;
    assert!(
        !sibling.contains(&cookie),
        "running sibling must not live-receive page A's cookie: {sibling}"
    );

    // Closing A merges its exit jar into the canonical snapshot...
    tabs_close(client, &page_a).await;

    // ...so a brand-new page C is seeded with the cookie.
    let _page_c = tabs_new(client, &base).await;
    let inherited = evaluate_str(client, "() => document.cookie").await;
    assert!(
        inherited.contains(&cookie),
        "new page must inherit the merged canonical cookie jar: {inherited}"
    );

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t14: stale refs after navigation/reload and after DOM mutation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t14_stale_refs_after_navigation_and_mutation() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("stale_refs_after_navigation_and_mutation") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;

    open_url(client, &wf.url).await;

    // After a reload the generation bumps: old refs fail closed.
    let snap = take_snapshot(client).await;
    let submit_ref = find_ref(&snap, "Submit");
    let reload = call(client, "browser_navigate", json!({ "action": "reload" })).await;
    assert_ok(&reload, "reload");
    let click = call(client, "browser_click", json!({ "ref": submit_ref })).await;
    assert_error_code(&click, ErrorCode::BrowserStaleRef, "click after reload");

    // DOM mutation without navigation: the rerender button replaces the
    // subtree. Trigger it via evaluate so the ref table is untouched, then a
    // ref to the replaced node must fail via nodeDetails-gone.
    let snap = take_snapshot(client).await;
    let inner_ref = find_ref(&snap, "Inner");
    let rerender = evaluate(
        client,
        "() => { document.getElementById('rerender-btn').click(); return 'rerendered'; }",
    )
    .await;
    assert_eq!(rerender, json!("rerendered"));
    let markdown = call(client, "browser_markdown", json!({ "ref": inner_ref })).await;
    assert_error_code(
        &markdown,
        ErrorCode::BrowserStaleRef,
        "ref to a replaced node",
    );

    // The node-gone failure invalidates the whole generation: every older ref
    // is now stale, even one pointing at a still-existing node.
    let snap_before = snap.clone();
    let rerender_ref = find_ref(&snap_before, "Rerender");
    let click = call(client, "browser_click", json!({ "ref": rerender_ref })).await;
    assert_error_code(
        &click,
        ErrorCode::BrowserStaleRef,
        "generation invalidated after node-gone",
    );

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t15: crash before an ambiguous click -> zero replay
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t15_crash_no_replay_side_effect_counter() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("crash_no_replay_side_effect_counter") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;

    open_url(client, &wf.url).await;
    let baseline = fixture_count(&wf.fixture).await;

    // One real side-effecting click; the out-of-band counter proves it ran.
    let snap = take_snapshot(client).await;
    let count_ref = find_ref(&snap, "Count");
    let click = call(client, "browser_click", json!({ "ref": count_ref })).await;
    assert_ok(&click, "first count click");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && fixture_count(&wf.fixture).await < baseline + 1 {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        fixture_count(&wf.fixture).await,
        baseline + 1,
        "first click executed exactly once"
    );

    // Fresh ref (the click invalidated the previous generation), then crash.
    let snap = take_snapshot(client).await;
    let count_ref = find_ref(&snap, "Count");
    let killed = env
        .browser
        .test_terminate_page_child(None)
        .await
        .expect("kill active page child");
    println!("terminated active page child pid {killed}");
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // The ambiguous second click must fail with BROWSER_PROCESS_LOST...
    let replayed = call(client, "browser_click", json!({ "ref": count_ref })).await;
    assert_error_code(
        &replayed,
        ErrorCode::BrowserProcessLost,
        "ambiguous click after crash",
    );

    // ...and must never be replayed against a restarted process.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        fixture_count(&wf.fixture).await,
        baseline + 1,
        "side-effecting click must never be replayed after process loss"
    );

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t16: unsupported tools fail closed with zero side effects
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t16_unsupported_tools_zero_side_effects() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("unsupported_tools_zero_side_effects") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;
    let base = wf.url.clone();

    open_url(client, &base).await;
    let count_before = fixture_count(&wf.fixture).await;
    let url_before = tabs_list(client)
        .await
        .into_iter()
        .find(|p| p["active"].as_bool().unwrap_or(false))
        .expect("active page")["url"]
        .as_str()
        .unwrap()
        .to_string();

    let unsupported: [(&str, Value); 8] = [
        ("browser_screenshot", json!({ "mode": "viewport" })),
        (
            "browser_set_viewport",
            json!({ "width": 800, "height": 600 }),
        ),
        (
            "browser_emulate_device",
            json!({ "emulation": { "source": "preset", "name": "iphone-x" } }),
        ),
        ("browser_dialog", json!({ "accept": true })),
        ("browser_upload", json!({ "sources": [] })),
        ("browser_downloads", json!({})),
        (
            "browser_save_download_to_workspace",
            json!({ "workspace_id": "w", "download_id": "d", "path": "p" }),
        ),
        ("browser_devtools", json!({})),
    ];
    for (tool, args) in unsupported {
        let result = call(client, tool, args).await;
        assert_error_code(
            &result,
            ErrorCode::BrowserFeatureUnsupported,
            &format!("{tool} must fail closed"),
        );
    }

    // Zero side effects: no fixture actions, no navigation, page still usable.
    assert_eq!(
        fixture_count(&wf.fixture).await,
        count_before,
        "unsupported tools must not trigger fixture side effects"
    );
    let url_after = tabs_list(client)
        .await
        .into_iter()
        .find(|p| p["active"].as_bool().unwrap_or(false))
        .expect("active page")["url"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(url_after, url_before, "unsupported tools must not navigate");
    let snap = take_snapshot(client).await;
    assert!(
        snap.contains("Submit"),
        "page remains usable after rejections"
    );

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t17: pause and disable terminate all page children
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t17_pause_and_disable_terminate_children() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("pause_and_disable_terminate_children") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;
    let base = wf.url.clone();

    // Two live pages.
    open_url(client, &base).await;
    let pages = tabs_list(client).await;
    let first_page = pages[0]["page_id"].as_str().unwrap().to_string();
    let second_page = tabs_new(client, &base).await;
    let pid_first = env
        .browser
        .test_page_child_pid(Some(&first_page))
        .await
        .expect("first pid");
    let pid_second = env
        .browser
        .test_page_child_pid(Some(&second_page))
        .await
        .expect("second pid");

    // Pause: the desktop kill switch path (pause flag + on_pause_changed)
    // must terminate EVERY page child.
    env.core.pause.set(true);
    env.browser
        .on_pause_changed(true)
        .await
        .expect("pause shutdown");
    wait_process_dead(pid_first, "first page child after pause").await;
    wait_process_dead(pid_second, "second page child after pause").await;

    // While paused, browser tools fail closed.
    let snap = call(client, "browser_snapshot", json!({})).await;
    assert_error_code(&snap, ErrorCode::AccessPaused, "snapshot while paused");

    // Unpause and start a fresh session for the disable half.
    env.core.pause.set(false);
    env.browser.on_pause_changed(false).await.expect("resume");
    open_url(client, &base).await;
    let pid_third = env
        .browser
        .test_page_child_pid(None)
        .await
        .expect("third pid");

    // Disable Browser Automation through the settings path: running children
    // must be terminated.
    env.browser
        .apply_settings(BrowserRuntimeSettings { enabled: false })
        .await
        .expect("disable settings");
    wait_process_dead(pid_third, "page child after disable").await;

    let open = call(client, "browser_open", json!({ "url": base })).await;
    assert_error_code(
        &open,
        ErrorCode::BrowserAutomationDisabled,
        "open while disabled",
    );

    // Re-enable so the environment ends in the default state.
    env.browser
        .apply_settings(BrowserRuntimeSettings { enabled: true })
        .await
        .expect("re-enable settings");
    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t18: reset clears only owned browser data
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t18_reset_clears_only_owned_browser_data() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("reset_clears_only_owned_browser_data") {
        return;
    }

    let wf = spawn_wsl_fixture().await;
    let env = spawn_real(background_settings()).await;
    let client = &env.client;
    let base = wf.url.clone();

    // Seed storage + cookies.
    open_url(client, &base).await;
    evaluate(client, "localStorage.setItem('reset_k', 'v')").await;
    evaluate(client, "document.cookie = 'reset18=1; path=/'; 'set'").await;
    close_browser(client, "browser_close before reset").await;

    // Reset only the app-owned browser data directories.
    env.runtime
        .reset_browser_data()
        .await
        .expect("reset owned browser data");

    // Distro, binary, and runtime state survive the reset.
    let snapshot = env.runtime.state_snapshot().expect("state snapshot");
    assert_eq!(snapshot.state, RuntimeProvisionState::Ready);
    let distro = snapshot.distro.expect("distro survives reset");
    assert!(
        wsl_distro_registered(&distro.distro_name),
        "distro must stay registered after reset"
    );

    // A fresh session starts with empty data (and proves the Lightpanda
    // binary still runs).
    open_url(client, &base).await;
    assert!(
        evaluate(client, "localStorage.getItem('reset_k')")
            .await
            .is_null(),
        "localStorage must be empty after reset"
    );
    let cookies = evaluate_str(client, "() => document.cookie").await;
    assert!(
        !cookies.contains("reset18"),
        "cookies must be empty after reset: {cookies}"
    );

    close_browser(client, "browser_close").await;
}

// ---------------------------------------------------------------------------
// t19: remove refuses ownership marker / install-ID mismatch
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t19_remove_refuses_ownership_mismatch() {
    let _guard = SERIAL.lock().await;
    if !require_runtime("remove_refuses_ownership_mismatch") {
        return;
    }

    // Ensure the real app-owned distro exists.
    let prod = production_runtime();
    prod.ensure_runtime().await.expect("runtime ready");
    let real = prod.distro_identity().expect("real distro identity");

    // A rogue manager whose persisted state CLAIMS the real distro name but
    // carries a foreign install id. Its remove_runtime must fail closed on
    // the ownership marker mismatch and leave the real distro untouched.
    let tmp = tempfile::tempdir().expect("tempdir");
    let wsl_dir = tmp.path().join("wsl");
    std::fs::create_dir_all(&wsl_dir).expect("create wsl dir");
    let seeded = json!({
        "state": "ready",
        "install_id": "deadbeef",
        "distro": {
            "distro_name": real.distro_name,
            "install_id": "deadbeef",
            "install_dir": wsl_dir.join(&real.distro_name),
        },
        "distro_image_version": null,
        "lightpanda_version": "0.3.7",
        "reason": null
    });
    std::fs::write(
        wsl_dir.join(RUNTIME_STATE_FILE),
        serde_json::to_string_pretty(&seeded).unwrap(),
    )
    .expect("seed rogue state file");

    let rogue = BrowserRuntimeManager::new(
        Arc::new(RealWslCommandRunner),
        Arc::new(HttpsRuntimeDownloader::new().expect("downloader")),
        Arc::new(RealRuntimeContractVerifier::new(PathBuf::from("wsl.exe"))),
        RuntimeManifest::from_json_str(&manifest_json()).expect("manifest"),
        tmp.path().to_path_buf(),
    );
    assert_eq!(
        rogue.current_state(),
        RuntimeProvisionState::Ready,
        "rogue manager must load the seeded state file"
    );

    let error = rogue
        .remove_runtime()
        .await
        .expect_err("remove must fail closed on ownership mismatch");
    assert_eq!(
        error.code,
        ErrorCode::BrowserWslDistroConflict,
        "ownership mismatch must surface BROWSER_WSL_DISTRO_CONFLICT: {error}"
    );

    // The real distro is untouched: still registered, install dir intact,
    // production state still Ready.
    assert!(
        wsl_distro_registered(&real.distro_name),
        "real distro must remain registered"
    );
    assert!(
        real.install_dir.exists(),
        "real install dir must remain intact: {}",
        real.install_dir.display()
    );
    assert_eq!(
        prod.state_snapshot().expect("prod snapshot").state,
        RuntimeProvisionState::Ready
    );
}
