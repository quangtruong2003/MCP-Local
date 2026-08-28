//! Real Lightpanda-in-managed-WSL2 runtime integration tests.
//!
//! Unlike `browser_mcp_integration.rs` (which drives a fake native caller),
//! this suite drives the REAL production path end-to-end:
//!
//! ```text
//! MCP client -> WorkspaceServer -> BrowserNativeCaller
//!   -> one LightpandaNativeService HTTP server
//!   -> native Lightpanda sessions via Mcp-Session-Id
//!   -> managed WSL2 Lightpanda 0.3.7
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
//! prefixed `t01_`..`t10_`; the suite serializes on a shared mutex because
//! every browser test touches the one managed distro.
//!
//! # 31-tool coverage proof
//!
//! Every public `browser_lightpanda_*` tool call goes through `tc`, which
//! records the tool name in `COVERAGE`. `t10` asserts the recorded set equals
//! the exact 31 expected public names — coverage is proven by actual calls,
//! never inferred from tool discovery alone.

#[path = "fixtures/browser_test_app.rs"]
mod browser_test_app;

use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use serde_json::{json, Value};

use lc_core::{error::ErrorCode, CoreServices};
use local_workspace_mcp_manager::{
    browser::{
        contract::{EXPECTED_NATIVE_TOOL_COUNT, NATIVE_TOOL_MAPPINGS, PINNED_LIGHTPANDA_VERSION},
        lightpanda::{LightpandaNativeService, NativeRuntimeManagerGate, WslNativeServerLauncher},
        runtime::{BrowserRuntimeManager, RuntimeProvisionState},
        types::BrowserRuntimeSettings,
        wsl::decode_wsl_output,
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

/// Serialize real-browser tests. Each launches an actual WSL-backed server
/// against the one shared managed distro; running them in parallel would
/// contend for the runtime. `--jobs 1` serializes test *binaries*, not the
/// threads inside one binary, so we serialize explicitly here.
static SERIAL: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Public tool names actually called by this suite (see `tc`).
static COVERAGE: std::sync::LazyLock<std::sync::Mutex<BTreeSet<&'static str>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(BTreeSet::new()));

/// Number of runtime-touching tests (t01..t09). `t10` waits for all of them
/// to finish before asserting the coverage table, independent of the order in
/// which libtest schedules the serialized tests.
const RUNTIME_TEST_COUNT: usize = 9;
static RUNTIME_TESTS_DONE: AtomicUsize = AtomicUsize::new(0);

/// Increments the done-counter even when the test panics.
struct DoneGuard;
impl Drop for DoneGuard {
    fn drop(&mut self) {
        RUNTIME_TESTS_DONE.fetch_add(1, Ordering::SeqCst);
    }
}

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
/// The browser server runs inside the WSL2 VM. It cannot reach the
/// Windows-side loopback fixture directly: WSL2 NAT has no VM->host
/// loopback forwarding, and Windows Firewall blocks inbound connections to
/// the freshly compiled test binary (the rule it creates is bound to one
/// binary hash and stops matching after every recompile). A tiny node TCP
/// proxy listening on the WSL-facing gateway IP bridges the two.
struct WslFixture {
    fixture: Fixture,
    /// Fixture URL reachable from inside the WSL2 VM (gateway host + proxy port).
    url: String,
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

/// Force-kills a process tree root by PID (Windows `taskkill`).
fn kill_pid(pid: u32) {
    let output = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output()
        .expect("taskkill executes");
    assert!(
        output.status.success(),
        "taskkill /F /PID {pid} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
    browser: Arc<LightpandaNativeService>,
    runtime: Arc<BrowserRuntimeManager>,
    _dir: tempfile::TempDir,
}

/// Spawns the full production MCP HTTP server wired exactly like the desktop
/// app: production runtime manager (real LocalAppData root) plus the single
/// native Lightpanda service. Nothing is provisioned or started until the
/// first native call passes the runtime gate.
async fn spawn_real(settings: AppSettings) -> RealEnv {
    let dir = tempfile::tempdir().expect("tempdir");
    save_app_settings(dir.path(), &settings).expect("save app settings");
    let core = Arc::new(CoreServices::open(&dir.path().join("test.db")).expect("open core"));
    let policy = Arc::new(AgentPolicyState::from_settings(&settings));
    let controller = Arc::new(AgentSettingsController::new(
        dir.path().to_path_buf(),
        policy.clone(),
    ));
    let runtime = production_runtime();
    let gate = Arc::new(NativeRuntimeManagerGate::new(runtime.clone()));
    let launcher = Arc::new(WslNativeServerLauncher::new(PathBuf::from("wsl.exe")));
    let browser =
        LightpandaNativeService::new(gate, launcher, BrowserRuntimeSettings::from(&settings));
    let capture: Arc<dyn ScreenCaptureProvider> = Arc::new(EmptyCapture);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mcp listener");
    let port = listener.local_addr().expect("mcp addr").port();
    let core_for_server = core.clone();
    let browser_for_server: Arc<
        dyn local_workspace_mcp_manager::browser::lightpanda::BrowserNativeCaller,
    > = browser.clone();
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
        _dir: dir,
    }
}

fn enabled_settings() -> AppSettings {
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

/// Tracked call: records the public tool name for the 31-tool coverage proof.
async fn tc(env: &RealEnv, tool: &'static str, args: Value) -> CallToolResult {
    COVERAGE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(tool);
    call(&env.client, tool, args).await
}

fn is_ok(result: &CallToolResult) -> bool {
    !result.is_error.unwrap_or(false)
}

fn result_text(result: &CallToolResult) -> String {
    serde_json::to_string(result).unwrap_or_default()
}

/// Concatenated text content of a native result.
fn first_text(result: &CallToolResult) -> String {
    let mut out = String::new();
    for block in result.content.iter() {
        if let Some(text) = block.as_text() {
            out.push_str(&text.text);
            out.push('\n');
        }
    }
    out
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

/// Adds `session_id` to an args object when routing to a named session.
fn with_session(args: Value, session: Option<&str>) -> Value {
    match session {
        None => args,
        Some(id) => {
            let mut map = args.as_object().expect("object args").clone();
            map.insert("session_id".into(), json!(id));
            Value::Object(map)
        }
    }
}

async fn goto(env: &RealEnv, url: &str, session: Option<&str>) {
    let result = tc(
        env,
        "browser_lightpanda_goto",
        with_session(json!({ "url": url }), session),
    )
    .await;
    assert_ok(&result, &format!("goto {url}"));
}

/// Evaluates page-context JavaScript and returns the native text output
/// (strings arrive bare/unquoted; wrap in JSON.stringify when structure is
/// needed).
async fn evaluate(env: &RealEnv, session: Option<&str>, script: &str) -> String {
    let result = tc(
        env,
        "browser_lightpanda_evaluate",
        with_session(json!({ "script": script }), session),
    )
    .await;
    assert_ok(&result, &format!("evaluate {script}"));
    first_text(&result).trim().to_string()
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
// Native output parsing helpers (tolerant of native text shapes)
// ---------------------------------------------------------------------------

/// Recursively finds a `backendNodeId` in a JSON value whose sibling string
/// fields mention `needle`.
fn find_node_id_json(value: &Value, needle: &str) -> Option<i64> {
    match value {
        Value::Object(map) => {
            let id = map.get("backendNodeId").and_then(Value::as_i64);
            let hit = map
                .values()
                .any(|v| v.as_str().is_some_and(|s| s.contains(needle)));
            if let (Some(id), true) = (id, hit) {
                return Some(id);
            }
            map.values().find_map(|v| find_node_id_json(v, needle))
        }
        Value::Array(items) => items.iter().find_map(|v| find_node_id_json(v, needle)),
        _ => None,
    }
}

/// Extracts the first integer following a backendNodeId-ish key on `line`.
fn extract_id_from_line(line: &str) -> Option<i64> {
    for key in ["backendNodeId", "backend_node_id", "backend node id"] {
        let Some(idx) = line.find(key) else { continue };
        let rest = &line[idx + key.len()..];
        let mut digits = String::new();
        let mut separators = 0;
        for c in rest.chars() {
            if c.is_ascii_digit() {
                digits.push(c);
            } else if !digits.is_empty() {
                break;
            } else {
                separators += 1;
                if separators > 4 {
                    break;
                }
            }
        }
        if let Ok(id) = digits.parse::<i64>() {
            return Some(id);
        }
    }
    None
}

/// Resolves the backendNodeId of the element described by `needle`, from
/// either JSON-shaped native output or line-oriented text where the id and
/// the element description share a line.
fn try_node_id_near(text: &str, needle: &str) -> Option<i64> {
    if let Ok(value) = serde_json::from_str::<Value>(text.trim()) {
        if let Some(id) = find_node_id_json(&value, needle) {
            return Some(id);
        }
    }
    for line in text.lines() {
        if line.contains(needle) {
            if let Some(id) = extract_id_from_line(line) {
                return Some(id);
            }
        }
    }
    None
}

/// Extracts the backendNodeId (leading token) from the semantic tree line
/// containing `needle`. Native tree lines look like `3 [i] textbox 'Name'`.
fn tree_node_id(tree_text: &str, needle: &str) -> i64 {
    for line in tree_text.lines() {
        if !line.contains(needle) {
            continue;
        }
        let digits: String = line
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(id) = digits.parse::<i64>() {
            return id;
        }
    }
    panic!("no tree node found near {needle:?} in native tree output:\n{tree_text}");
}

/// Resolves the backendNodeId of the element with the given DOM id through
/// the native interactiveElements tool. Used for listener-attached elements
/// (like the hover target) that are listed there but not named in the
/// semantic tree.
async fn interactive_element_id_by_dom_id(env: &RealEnv, dom_id: &str) -> i64 {
    let result = tc(env, "browser_lightpanda_interactive_elements", json!({})).await;
    assert_ok(&result, "interactiveElements");
    let text = first_text(&result);
    let value: Value = serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("interactiveElements must be JSON: {e}\n{text}"));
    for item in value.as_array().expect("interactiveElements array") {
        if item.get("id").and_then(Value::as_str) == Some(dom_id) {
            if let Some(id) = item.get("backendNodeId").and_then(Value::as_i64) {
                return id;
            }
        }
    }
    panic!("no interactive element with DOM id {dom_id:?} in: {text}");
}

/// Like `try_node_id_near`, but panics with the full text when the id cannot
/// be found, so format drift is visible in test output.
fn node_id_near(text: &str, needle: &str) -> i64 {
    try_node_id_near(text, needle).unwrap_or_else(|| {
        panic!("no backendNodeId found near {needle:?} in native output:\n{text}")
    })
}

// ---------------------------------------------------------------------------
// Release evidence (consumed by scripts/verify-browser-release.ps1)
// ---------------------------------------------------------------------------

/// Prints the explicit parity evidence lines for the release gate and asserts
/// they are all PASS. Called once, after the first native server start.
async fn print_parity_evidence(env: &RealEnv) {
    let status = env.browser.status().await;
    let tools = env.client.list_all_tools().await.expect("outer discovery");
    let public = tools
        .iter()
        .filter(|t| t.name.starts_with("browser_lightpanda_"))
        .count();
    let legacy = tools
        .iter()
        .filter(|t| t.name.starts_with("browser_") && !t.name.starts_with("browser_lightpanda_"))
        .count();
    println!("Lightpanda version: {PINNED_LIGHTPANDA_VERSION}");
    println!("Native tools discovered: {}", status.native_tool_count);
    println!("Native tools expected: {EXPECTED_NATIVE_TOOL_COUNT}");
    println!("Public browser_lightpanda tools: {public}");
    println!("Legacy browser tools discovered: {legacy}");
    let pass = status.contract_verified
        && status.native_tool_count == EXPECTED_NATIVE_TOOL_COUNT
        && public == EXPECTED_NATIVE_TOOL_COUNT
        && legacy == 0;
    println!("Contract parity: {}", if pass { "PASS" } else { "FAIL" });
    assert!(pass, "contract parity evidence must be PASS");
}

// ---------------------------------------------------------------------------
// t01: provisioning reports the pinned binary version (not serverInfo)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t01_runtime_provisioning_reports_pinned_binary_version() {
    let _done = DoneGuard;
    let _guard = SERIAL.lock().await;
    if !require_runtime("runtime_provisioning_reports_pinned_binary_version") {
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
    // The binary pin comes from the runtime manager's in-distro
    // `lightpanda version` verification, persisted into the state file.
    // Regression guard: the MCP protocol's initialize serverInfo is NEVER
    // the binary pin — contract verification compares tools/list against the
    // frozen snapshot, and this suite never asserts serverInfo.version.
    assert_eq!(
        snapshot.lightpanda_version.as_deref(),
        Some(PINNED_LIGHTPANDA_VERSION),
        "Lightpanda must stay pinned to {PINNED_LIGHTPANDA_VERSION}"
    );
    assert!(
        wsl_distro_registered(&distro.distro_name),
        "managed distro must be registered with WSL: {}",
        distro.distro_name
    );
}

// ---------------------------------------------------------------------------
// t02: bootstrap session list, named-session isolation, one server process
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t02_bootstrap_sessions_single_server_process_and_parity_evidence() {
    let _done = DoneGuard;
    let _guard = SERIAL.lock().await;
    if !require_runtime("bootstrap_sessions_single_server_process") {
        return;
    }

    let env = spawn_real(enabled_settings()).await;
    let wsl_fixture = spawn_wsl_fixture().await;

    // The very first native call: only `default` may exist. This proves
    // initialization did not leak an internal `s1` session.
    let list = tc(&env, "browser_lightpanda_session_list", json!({})).await;
    assert_ok(&list, "bootstrap session_list");
    let list_text = first_text(&list);
    assert!(
        list_text.contains("default"),
        "session_list must report the default session: {list_text}"
    );
    assert!(
        !list_text.contains("alpha") && !list_text.contains("s1"),
        "bootstrap session_list leaked a non-default session: {list_text}"
    );
    let pid_after_start = env
        .browser
        .server_child_pid()
        .await
        .expect("server child running after first native call");

    // Create two named sessions and navigate them independently.
    let alpha = tc(
        &env,
        "browser_lightpanda_session_new",
        json!({ "name": "alpha" }),
    )
    .await;
    assert_ok(&alpha, "session_new alpha");
    assert!(
        first_text(&alpha).contains("alpha"),
        "session_new must return the session id: {}",
        first_text(&alpha)
    );
    let beta = tc(
        &env,
        "browser_lightpanda_session_new",
        json!({ "name": "beta" }),
    )
    .await;
    assert_ok(&beta, "session_new beta");

    goto(&env, &wsl_fixture.url, Some("alpha")).await;
    goto(&env, &format!("{}/plain", wsl_fixture.url), Some("beta")).await;

    let url_alpha = tc(
        &env,
        "browser_lightpanda_get_url",
        with_session(json!({}), Some("alpha")),
    )
    .await;
    assert_ok(&url_alpha, "get_url alpha");
    let url_alpha = first_text(&url_alpha);
    assert!(
        url_alpha.trim().starts_with(&wsl_fixture.url) && !url_alpha.contains("/plain"),
        "alpha must stay on the fixture root: {url_alpha}"
    );

    let url_beta = tc(
        &env,
        "browser_lightpanda_get_url",
        with_session(json!({}), Some("beta")),
    )
    .await;
    assert_ok(&url_beta, "get_url beta");
    let url_beta = first_text(&url_beta);
    assert!(
        url_beta.trim().ends_with("/plain"),
        "beta must be on /plain independently of alpha: {url_beta}"
    );

    // One shared server process serves every session: the child PID never
    // changes across session control and per-session navigation calls.
    let pid_now = env
        .browser
        .server_child_pid()
        .await
        .expect("server child still running");
    assert_eq!(
        pid_after_start, pid_now,
        "exactly one server process must serve all named sessions"
    );

    // Release evidence for the Task 10 gate.
    print_parity_evidence(&env).await;

    // Closing a named session removes it from the list.
    let close = tc(
        &env,
        "browser_lightpanda_session_close",
        json!({ "id": "beta" }),
    )
    .await;
    assert_ok(&close, "session_close beta");
    let list = tc(&env, "browser_lightpanda_session_list", json!({})).await;
    let list_text = first_text(&list);
    assert!(
        list_text.contains("alpha") && !list_text.contains("beta"),
        "beta must be gone after session_close: {list_text}"
    );
}

// ---------------------------------------------------------------------------
// t03: content/metadata tools on the fixture (default session)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t03_content_and_metadata_tools_on_fixture() {
    let _done = DoneGuard;
    let _guard = SERIAL.lock().await;
    if !require_runtime("content_and_metadata_tools_on_fixture") {
        return;
    }

    let env = spawn_real(enabled_settings()).await;
    let wsl_fixture = spawn_wsl_fixture().await;
    goto(&env, &wsl_fixture.url, None).await;

    let markdown = tc(&env, "browser_lightpanda_markdown", json!({})).await;
    assert_ok(&markdown, "markdown");
    let markdown_text = first_text(&markdown);
    assert!(
        markdown_text.contains("Browser Test App"),
        "markdown must contain the page heading: {markdown_text}"
    );

    let html = tc(&env, "browser_lightpanda_html", json!({})).await;
    assert_ok(&html, "html");
    let html_text = first_text(&html);
    assert!(
        html_text.contains("name-input"),
        "html must contain the fixture markup: {html_text}"
    );

    let links = tc(&env, "browser_lightpanda_links", json!({})).await;
    assert_ok(&links, "links");
    let links_text = first_text(&links);
    assert!(
        links_text.contains("PlainPage") && links_text.contains("/plain"),
        "links must include the /plain anchor: {links_text}"
    );
    // Links carry raw backendNodeIds usable for click/nodeDetails.
    let plain_link_id = node_id_near(&links_text, "/plain");
    assert!(plain_link_id > 0, "link backendNodeId must be positive");

    let tree = tc(&env, "browser_lightpanda_tree", json!({})).await;
    assert_ok(&tree, "tree");
    let tree_text = first_text(&tree);
    assert!(
        tree_text.contains("Submit"),
        "tree must include the Submit button: {tree_text}"
    );

    let node = tc(
        &env,
        "browser_lightpanda_node_details",
        json!({ "backendNodeId": plain_link_id }),
    )
    .await;
    assert_ok(&node, "nodeDetails for the /plain link");
    let node_text = first_text(&node);
    assert!(
        node_text.contains("plain-link") || node_text.contains("/plain"),
        "nodeDetails must describe the /plain anchor: {node_text}"
    );

    let interactive = tc(&env, "browser_lightpanda_interactive_elements", json!({})).await;
    assert_ok(&interactive, "interactiveElements");
    let interactive_text = first_text(&interactive);
    assert!(
        interactive_text.contains("Submit"),
        "interactiveElements must list the Submit button: {interactive_text}"
    );

    let structured = tc(&env, "browser_lightpanda_structured_data", json!({})).await;
    assert_ok(&structured, "structuredData");
    let structured_text = first_text(&structured);
    assert!(
        structured_text.contains("Fixture JsonLd Page"),
        "structuredData must include JSON-LD: {structured_text}"
    );
    assert!(
        structured_text.contains("Fixture OpenGraph Title"),
        "structuredData must include OpenGraph: {structured_text}"
    );

    let forms = tc(&env, "browser_lightpanda_detect_forms", json!({})).await;
    assert_ok(&forms, "detectForms");
    let forms_text = first_text(&forms);
    assert!(
        forms_text.contains("inputType") && forms_text.contains("options"),
        "detectForms must describe the fixture form fields: {forms_text}"
    );

    let found = tc(
        &env,
        "browser_lightpanda_find_element",
        json!({ "role": "button", "name": "Submit" }),
    )
    .await;
    assert_ok(&found, "findElement Submit button");
    let found_text = first_text(&found);
    assert!(
        found_text.contains("Submit"),
        "findElement must match the Submit button: {found_text}"
    );
    assert!(
        try_node_id_near(&found_text, "Submit").is_some(),
        "findElement must return a backendNodeId: {found_text}"
    );

    // Console buffer is read-once: call it last in this session.
    let logs = tc(&env, "browser_lightpanda_console_logs", json!({})).await;
    assert_ok(&logs, "consoleLogs");
    let logs_text = first_text(&logs);
    assert!(
        logs_text.contains("known-console-log"),
        "consoleLogs must include the fixture console output: {logs_text}"
    );
}

// ---------------------------------------------------------------------------
// t04: raw backendNodeId action parity (no selector recovery)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t04_raw_backend_node_id_action_parity() {
    let _done = DoneGuard;
    let _guard = SERIAL.lock().await;
    if !require_runtime("raw_backend_node_id_action_parity") {
        return;
    }

    let env = spawn_real(enabled_settings()).await;
    let wsl_fixture = spawn_wsl_fixture().await;
    goto(&env, &wsl_fixture.url, None).await;

    // Resolve raw backendNodeIds from the native semantic tree (leading
    // token per line). Listener-attached elements are unnamed in the tree
    // (or only listed by their text child), so they are resolved through
    // interactiveElements by DOM id.
    let tree = tc(&env, "browser_lightpanda_tree", json!({})).await;
    assert_ok(&tree, "tree");
    let tree_text = first_text(&tree);
    let name_id = tree_node_id(&tree_text, "textbox 'Name'");
    let color_id = tree_node_id(&tree_text, "combobox 'Color'");
    let agree_id = tree_node_id(&tree_text, "checkbox 'Agree'");
    let large_id = tree_node_id(&tree_text, "radio 'Large'");
    let submit_id = tree_node_id(&tree_text, "button 'Submit'");
    let keyboard_id = tree_node_id(&tree_text, "textbox 'Keyboard'");
    let hover_id = interactive_element_id_by_dom_id(&env, "hover-target").await;
    let bottom_id = interactive_element_id_by_dom_id(&env, "scroll-bottom").await;

    // Raw ids remain present in native tree output (no id rewriting).
    for id in [name_id, color_id, submit_id] {
        assert!(
            tree_text.contains(&id.to_string()),
            "raw backendNodeId {id} must appear unchanged in native tree output"
        );
    }

    // fill by raw id.
    let fill = tc(
        &env,
        "browser_lightpanda_fill",
        json!({ "backendNodeId": name_id, "value": "Lightpanda" }),
    )
    .await;
    assert_ok(&fill, "fill by backendNodeId");
    assert_eq!(
        evaluate(&env, None, "document.getElementById('name-input').value").await,
        "Lightpanda"
    );

    // selectOption by raw id.
    let select = tc(
        &env,
        "browser_lightpanda_select_option",
        json!({ "backendNodeId": color_id, "value": "green" }),
    )
    .await;
    assert_ok(&select, "selectOption by backendNodeId");
    assert_eq!(
        evaluate(&env, None, "document.getElementById('color-select').value").await,
        "green"
    );

    // setChecked by raw id.
    let check = tc(
        &env,
        "browser_lightpanda_set_checked",
        json!({ "backendNodeId": agree_id, "checked": true }),
    )
    .await;
    assert_ok(&check, "setChecked by backendNodeId");
    assert_eq!(
        evaluate(
            &env,
            None,
            "document.getElementById('agree-checkbox').checked"
        )
        .await,
        "true"
    );

    // click a radio by raw id.
    let radio = tc(
        &env,
        "browser_lightpanda_click",
        json!({ "backendNodeId": large_id }),
    )
    .await;
    assert_ok(&radio, "click radio by backendNodeId");
    assert_eq!(
        evaluate(&env, None, "document.getElementById('size-large').checked").await,
        "true"
    );

    // hover by raw id.
    let hover = tc(
        &env,
        "browser_lightpanda_hover",
        json!({ "backendNodeId": hover_id }),
    )
    .await;
    assert_ok(&hover, "hover by backendNodeId");
    assert_eq!(
        evaluate(
            &env,
            None,
            "document.getElementById('hover-target').textContent"
        )
        .await,
        "hover:hovered"
    );

    // press by raw id.
    let press = tc(
        &env,
        "browser_lightpanda_press",
        json!({ "key": "a", "backendNodeId": keyboard_id }),
    )
    .await;
    assert_ok(&press, "press by backendNodeId");
    assert!(
        evaluate(
            &env,
            None,
            "document.getElementById('keyboard-log').textContent"
        )
        .await
        .contains('a'),
        "keyboard target must receive the pressed key"
    );

    // click the submit button by raw id; the page composes fill+select state.
    let click = tc(
        &env,
        "browser_lightpanda_click",
        json!({ "backendNodeId": submit_id }),
    )
    .await;
    assert_ok(&click, "click submit by backendNodeId");
    assert_eq!(
        evaluate(&env, None, "document.getElementById('result').textContent").await,
        "result:name=Lightpanda,color=green"
    );

    // Page scroll, then scroll a specific node into view by raw id.
    let scroll = tc(&env, "browser_lightpanda_scroll", json!({ "y": 2000 })).await;
    assert_ok(&scroll, "scroll page by y offset");
    let scroll_y: f64 = evaluate(&env, None, "window.scrollY")
        .await
        .parse()
        .expect("scrollY is a number");
    assert!(scroll_y >= 1500.0, "page must actually scroll: {scroll_y}");
    let scroll_node = tc(
        &env,
        "browser_lightpanda_scroll",
        json!({ "backendNodeId": bottom_id }),
    )
    .await;
    assert_ok(&scroll_node, "scroll node by backendNodeId");
    let scroll_node_text = first_text(&scroll_node);
    assert!(
        scroll_node_text.contains("Scrolled"),
        "element scroll must report the native scroll position: {scroll_node_text}"
    );

    // Subtree calls scoped by raw backendNodeId.
    let sub_markdown = tc(
        &env,
        "browser_lightpanda_markdown",
        json!({ "backendNodeId": submit_id }),
    )
    .await;
    assert_ok(&sub_markdown, "markdown subtree by backendNodeId");
    assert!(first_text(&sub_markdown).contains("Submit"));
    let sub_html = tc(
        &env,
        "browser_lightpanda_html",
        json!({ "backendNodeId": submit_id }),
    )
    .await;
    assert_ok(&sub_html, "html subtree by backendNodeId");
    let sub_html_text = first_text(&sub_html);
    assert!(
        sub_html_text.contains("button") && sub_html_text.contains("Submit"),
        "html subtree must be scoped to the button: {sub_html_text}"
    );
    let sub_tree = tc(
        &env,
        "browser_lightpanda_tree",
        json!({ "backendNodeId": submit_id }),
    )
    .await;
    assert_ok(&sub_tree, "tree subtree by backendNodeId");
    assert!(first_text(&sub_tree).contains("Submit"));
}

// ---------------------------------------------------------------------------
// t05: selector-based replay + PandaScript save (with escape rejection)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t05_selector_actions_and_pandascript_save() {
    let _done = DoneGuard;
    let _guard = SERIAL.lock().await;
    if !require_runtime("selector_actions_and_pandascript_save") {
        return;
    }

    let env = spawn_real(enabled_settings()).await;
    let wsl_fixture = spawn_wsl_fixture().await;
    goto(&env, &wsl_fixture.url, None).await;

    // Representative actions repeated with CSS selectors (replayable form).
    let fill = tc(
        &env,
        "browser_lightpanda_fill",
        json!({ "selector": "#name-input", "value": "Replayed" }),
    )
    .await;
    assert_ok(&fill, "fill by selector");
    let select = tc(
        &env,
        "browser_lightpanda_select_option",
        json!({ "selector": "#color-select", "value": "blue" }),
    )
    .await;
    assert_ok(&select, "selectOption by selector");
    let click = tc(
        &env,
        "browser_lightpanda_click",
        json!({ "selector": "#submit-btn" }),
    )
    .await;
    assert_ok(&click, "click by selector");
    assert_eq!(
        evaluate(&env, None, "document.getElementById('result').textContent").await,
        "result:name=Replayed,color=blue"
    );

    // Save a deterministic replayable PandaScript for this session.
    let script = format!(
        "const page = new Page();\nawait page.goto(\"{}\");\n\
         await page.fill({{ selector: \"#name-input\", value: \"Replayed\" }});\n\
         await page.selectOption({{ selector: \"#color-select\", value: \"blue\" }});\n\
         await page.click({{ selector: \"#submit-btn\" }});\n",
        wsl_fixture.url
    );
    // Lightpanda `save` does not create parent directories: pre-create the
    // fixed integration subdirectory inside the PandaScript root first.
    // Runs as the runtime user so the directory is writable by the server.
    let distro = env
        .runtime
        .distro_identity()
        .expect("distro identity")
        .distro_name;
    let mkdir = std::process::Command::new("wsl.exe")
        .args([
            "-d",
            &distro,
            "-u",
            "localworkspaces",
            "--",
            "mkdir",
            "-p",
            "/var/lib/local-workspaces/browser/pandascripts/integration",
        ])
        .output()
        .expect("wsl.exe mkdir");
    assert!(
        mkdir.status.success(),
        "integration PandaScript subdirectory must be creatable"
    );
    let save = tc(
        &env,
        "browser_lightpanda_save",
        json!({ "path": "integration/replayable.js", "script": script }),
    )
    .await;
    assert_ok(&save, "save PandaScript");
    let save_text = first_text(&save);
    assert!(
        save_text.contains("/var/lib/local-workspaces/browser/pandascripts"),
        "save must report the fixed PandaScript directory: {save_text}"
    );

    // Path traversal must fail closed and never escape the fixed cwd.
    let escape = tc(
        &env,
        "browser_lightpanda_save",
        json!({ "path": "../escape.js", "script": "const page = new Page();\n" }),
    )
    .await;
    assert!(
        escape.is_error.unwrap_or(false),
        "save with a ../ path must return isError: {}",
        result_text(&escape)
    );
    let escaped = std::process::Command::new("wsl.exe")
        .args([
            "-d",
            &distro,
            "--",
            "test",
            "-f",
            "/var/lib/local-workspaces/browser/escape.js",
        ])
        .output()
        .expect("wsl.exe test")
        .status
        .success();
    assert!(
        !escaped,
        "../escape.js must not land outside the PandaScript directory"
    );

    // App-owned cleanup through a fixed wsl.exe shape (no browser-tool
    // filesystem access).
    let _ = std::process::Command::new("wsl.exe")
        .args([
            "-d",
            &distro,
            "--",
            "rm",
            "-rf",
            "/var/lib/local-workspaces/browser/pandascripts/integration",
        ])
        .output();
}

// ---------------------------------------------------------------------------
// t06: wait tools, evaluate semantics, extract
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t06_wait_tools_evaluate_and_extract() {
    let _done = DoneGuard;
    let _guard = SERIAL.lock().await;
    if !require_runtime("wait_tools_evaluate_and_extract") {
        return;
    }

    let env = spawn_real(enabled_settings()).await;
    let wsl_fixture = spawn_wsl_fixture().await;
    goto(&env, &wsl_fixture.url, None).await;

    // The fixture adds #delayed-element after 300ms and sets
    // window.__delayedState after 500ms.
    let wait_selector = tc(
        &env,
        "browser_lightpanda_wait_for_selector",
        json!({ "selector": "#delayed-element", "timeout": 10000 }),
    )
    .await;
    assert_ok(&wait_selector, "waitForSelector delayed element");

    let wait_script = tc(
        &env,
        "browser_lightpanda_wait_for_script",
        json!({ "script": "window.__delayedState === 'ready'", "timeout": 10000 }),
    )
    .await;
    assert_ok(&wait_script, "waitForScript delayed state");

    let wait_state = tc(
        &env,
        "browser_lightpanda_wait_for_state",
        json!({ "state": "load" }),
    )
    .await;
    assert_ok(&wait_state, "waitForState load");

    // Native evaluate output shape: strings arrive bare/unquoted.
    assert_eq!(
        evaluate(
            &env,
            None,
            "document.getElementById('extract-target').getAttribute('data-sku')"
        )
        .await,
        "SKU-42"
    );

    let extract = tc(
        &env,
        "browser_lightpanda_extract",
        json!({ "schema": "{\"item\":\"#extract-target\",\"heading\":\"#main-heading\"}" }),
    )
    .await;
    assert_ok(&extract, "extract");
    let extract_text = first_text(&extract);
    assert!(
        extract_text.contains("ExtractItem") && extract_text.contains("Browser Test App"),
        "extract must follow the CSS-selector schema: {extract_text}"
    );
}

// ---------------------------------------------------------------------------
// t07: cookie isolation between native sessions
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t07_cookie_isolation_between_native_sessions() {
    let _done = DoneGuard;
    let _guard = SERIAL.lock().await;
    if !require_runtime("cookie_isolation_between_native_sessions") {
        return;
    }

    let env = spawn_real(enabled_settings()).await;
    let wsl_fixture = spawn_wsl_fixture().await;

    let alpha = tc(
        &env,
        "browser_lightpanda_session_new",
        json!({ "name": "alpha" }),
    )
    .await;
    assert_ok(&alpha, "session_new alpha");
    let beta = tc(
        &env,
        "browser_lightpanda_session_new",
        json!({ "name": "beta" }),
    )
    .await;
    assert_ok(&beta, "session_new beta");

    // Alpha visits the cookie-setting page.
    goto(&env, &format!("{}/cookie", wsl_fixture.url), Some("alpha")).await;
    let cookies_alpha = tc(
        &env,
        "browser_lightpanda_get_cookies",
        with_session(json!({}), Some("alpha")),
    )
    .await;
    assert_ok(&cookies_alpha, "getCookies in alpha");
    let alpha_text = first_text(&cookies_alpha);
    assert!(
        alpha_text.contains("fixture-cookie=chocolate"),
        "alpha must see the cookie set by its own navigation: {alpha_text}"
    );

    // Beta browses the same host but must NOT see alpha's cookie: native
    // sessions are isolated and Local Workspaces never merges cookies to
    // emulate old tabs.
    goto(&env, &wsl_fixture.url, Some("beta")).await;
    let cookies_beta = tc(
        &env,
        "browser_lightpanda_get_cookies",
        with_session(json!({}), Some("beta")),
    )
    .await;
    assert_ok(&cookies_beta, "getCookies in beta");
    let beta_text = first_text(&cookies_beta);
    assert!(
        !beta_text.contains("fixture-cookie"),
        "beta must not see alpha's cookie: {beta_text}"
    );
}

// ---------------------------------------------------------------------------
// t08: getEnv list mode + bounded search smoke
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t08_getenv_list_and_search_smoke() {
    let _done = DoneGuard;
    let _guard = SERIAL.lock().await;
    if !require_runtime("getenv_list_and_search_smoke") {
        return;
    }

    let env = spawn_real(enabled_settings()).await;
    let wsl_fixture = spawn_wsl_fixture().await;
    goto(&env, &wsl_fixture.url, None).await;

    // List mode: names of set LP_* variables only. No secret LP_* variable
    // is required to exist; a successful (possibly empty) list is the
    // assertion.
    let env_result = tc(&env, "browser_lightpanda_get_env", json!({})).await;
    assert_ok(&env_result, "getEnv list mode");

    // Bounded smoke: the native search tool must answer with a valid native
    // result or a native error (e.g. rate limiting) — never a transport
    // failure. Ranking/content is deliberately not asserted.
    let search = tc(
        &env,
        "browser_lightpanda_search",
        json!({ "query": "lightpanda headless browser", "timeout": 20000 }),
    )
    .await;
    let search_text = first_text(&search);
    assert!(
        !search_text.trim().is_empty(),
        "search must return native content on either path: {}",
        result_text(&search)
    );
    println!(
        "search smoke: is_error={:?} (valid native result/error path)",
        search.is_error
    );
}

// ---------------------------------------------------------------------------
// t09: lifecycle — Pause, disable, re-enable, process loss without replay
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t09_lifecycle_pause_disable_and_process_loss_no_replay() {
    let _done = DoneGuard;
    let _guard = SERIAL.lock().await;
    if !require_runtime("lifecycle_pause_disable_and_process_loss_no_replay") {
        return;
    }

    let env = spawn_real(enabled_settings()).await;
    let wsl_fixture = spawn_wsl_fixture().await;
    goto(&env, &wsl_fixture.url, None).await;
    let pid1 = env
        .browser
        .server_child_pid()
        .await
        .expect("server running after first call");

    // Pause is authoritative: it stops the single server.
    env.browser.on_pause_changed(true).await;
    let status = env.browser.status().await;
    assert!(!status.running, "pause must stop the native server");
    assert!(env.browser.server_child_pid().await.is_none());
    wait_process_dead(pid1, "paused server child").await;

    let paused = tc(&env, "browser_lightpanda_get_url", json!({})).await;
    assert_error_code(
        &paused,
        ErrorCode::BrowserAutomationDisabled,
        "call while paused",
    );

    // Unpaused but disabled: calls fail closed and never restart the server.
    env.browser.on_pause_changed(false).await;
    env.browser
        .apply_settings(BrowserRuntimeSettings { enabled: false })
        .await;
    let disabled = tc(&env, "browser_lightpanda_get_url", json!({})).await;
    assert_error_code(
        &disabled,
        ErrorCode::BrowserAutomationDisabled,
        "call while disabled",
    );
    assert!(
        !env.browser.status().await.running,
        "disabled Browser Automation must not restart the server"
    );

    // Re-enable allows a lazy restart on the next independent call.
    env.browser
        .apply_settings(BrowserRuntimeSettings { enabled: true })
        .await;
    goto(&env, &wsl_fixture.url, None).await;
    let pid2 = env
        .browser
        .server_child_pid()
        .await
        .expect("server restarted after re-enable");

    // Baseline side effect: exactly one Count click reaches the fixture.
    let click = tc(
        &env,
        "browser_lightpanda_click",
        json!({ "selector": "#count-btn" }),
    )
    .await;
    assert_ok(&click, "count click before process loss");
    let deadline = Instant::now() + Duration::from_secs(10);
    while fixture_count(&wsl_fixture.fixture).await < 1 {
        assert!(
            Instant::now() < deadline,
            "count click never reached the fixture"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Induce process death out-of-band, then verify the interrupted call
    // fails and is NEVER replayed by the lazy restart.
    kill_pid(pid2);
    wait_process_dead(pid2, "killed server child").await;
    let lost = tc(
        &env,
        "browser_lightpanda_click",
        json!({ "selector": "#count-btn" }),
    )
    .await;
    assert!(
        lost.is_error.unwrap_or(false),
        "call against a dead server must fail: {}",
        result_text(&lost)
    );
    assert_eq!(
        fixture_count(&wsl_fixture.fixture).await,
        1,
        "the failed click must not be replayed against the fixture"
    );

    // The next independent call lazily starts a fresh server. A fresh
    // default session has no page loaded yet, so the restart call is a goto
    // (get_url would correctly report FrameNotLoaded on the new session).
    goto(&env, &wsl_fixture.url, None).await;
    assert!(
        env.browser.server_child_pid().await.is_some(),
        "a fresh server child must run after restart"
    );
    assert_eq!(
        fixture_count(&wsl_fixture.fixture).await,
        1,
        "restart itself must not replay any interrupted call"
    );
}

// ---------------------------------------------------------------------------
// t10: the coverage table equals the exact 31 public native tools
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t10_coverage_table_is_exact_native_31() {
    if !require_runtime("coverage_table_is_exact_native_31") {
        return;
    }

    // Wait until every runtime test has finished (they serialize on SERIAL;
    // this loop makes the assertion independent of scheduling order).
    let covered = loop {
        {
            let _guard = SERIAL.lock().await;
            if RUNTIME_TESTS_DONE.load(Ordering::SeqCst) >= RUNTIME_TEST_COUNT {
                break COVERAGE
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    let expected: BTreeSet<&'static str> = NATIVE_TOOL_MAPPINGS
        .iter()
        .map(|(_, public)| *public)
        .collect();
    assert_eq!(
        expected.len(),
        EXPECTED_NATIVE_TOOL_COUNT,
        "contract mapping must define exactly 31 public tools"
    );
    let missing: Vec<_> = expected.difference(&covered).collect();
    let unexpected: Vec<_> = covered.difference(&expected).collect();
    assert!(
        missing.is_empty() && unexpected.is_empty(),
        "31-tool coverage mismatch.\nmissing: {missing:?}\nunexpected: {unexpected:?}"
    );
    println!(
        "native tool coverage: {}/{} public browser_lightpanda tools called by the suite",
        covered.len(),
        EXPECTED_NATIVE_TOOL_COUNT
    );
}
