//! Private Playwright MCP child-process adapter.
//!
//! The upstream server is never exposed on a TCP port. Local Workspaces owns
//! the only client connection over child stdio and forwards a deliberately
//! restricted tool set.

use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use lc_core::error::{CoreError, ErrorCode};
use rmcp::{
    model::{CallToolRequestParams, CallToolResult},
    service::{RoleClient, RunningService},
    transport::TokioChildProcess,
    ServiceExt,
};
use serde_json::{json, Map, Value};
use tokio::{io::AsyncBufReadExt, sync::Mutex};

use crate::process_job::JobHandle;

use super::{
    driver::{BrowserDriver, BrowserDriverStatus},
    types::{BrowserEngine, BrowserLaunchSpec, BrowserRunMode},
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PLAYWRIGHT_MCP_VERSION: &str = "0.0.79";

/// RCE-equivalent upstream tools must never be reachable via generic proxying.
const FORBIDDEN_PROCESS_CODE_TOOLS: &[&str] = &["browser_run_code", "browser_run_code_unsafe"];

/// Upstream methods Local Workspaces intentionally composes. Storage/cookies,
/// routes, arbitrary network bodies, coordinate input, and unsafe code are not
/// exposed by this adapter.
///
/// Note: `browser_navigate_forward` and `browser_reload` are intentionally NOT
/// listed. The pinned upstream runtime (@playwright/mcp 0.0.79) marks them
/// `skillOnly`, so they are never exposed over MCP; requiring/forwarding them
/// would fail startup and mid-session respectively.
pub const GENERIC_TOOL_ALLOWLIST: &[&str] = &[
    "browser_navigate",
    "browser_navigate_back",
    "browser_snapshot",
    "browser_click",
    "browser_hover",
    "browser_drag",
    "browser_type",
    "browser_select_option",
    "browser_press_key",
    "browser_handle_dialog",
    "browser_file_upload",
    "browser_resize",
    "browser_take_screenshot",
    "browser_console_messages",
    "browser_network_requests",
    "browser_evaluate",
    "browser_tabs",
    "browser_wait_for",
    "browser_close",
    "browser_start_tracing",
    "browser_stop_tracing",
];

/// The sole process-level code path. This string is application-owned and has
/// no interpolation point for MCP/user input.
const CHROMIUM_PERFORMANCE_CODE: &str = r#"async (page) => {
  const session = await page.context().newCDPSession(page);
  await session.send('Performance.enable');
  const result = await session.send('Performance.getMetrics');
  await session.detach();
  return result;
}"#;

/// Fixed app-owned page-context expressions. These run through upstream
/// `browser_evaluate` (page context) and never through process-level code
/// execution. They are named constants so tests can prove that model/MCP input
/// is never interpolated into them.
pub(crate) const WAIT_VISIBLE_FUNCTION: &str = "(element) => { const s=getComputedStyle(element); const r=element.getBoundingClientRect(); return s.visibility !== 'hidden' && s.display !== 'none' && r.width > 0 && r.height > 0; }";
pub(crate) const WAIT_URL_FUNCTION: &str = "() => location.href";
pub(crate) const WAIT_READY_FUNCTION: &str = "() => document.readyState";

/// DOM inspection expression. The outerHTML is serialized from a CLONED element
/// after redacting sensitive attributes and password values, so the live DOM is
/// never mutated and secret values never leak through the serialized markup.
pub(crate) const DOM_INSPECT_FUNCTION: &str = r#"(element) => {
  const sensitive = (name) => {
    const n = String(name || '').toLowerCase();
    return n.includes('token') || n.includes('secret') || n.includes('password') || n === 'value';
  };
  const r = element.getBoundingClientRect();
  const s = getComputedStyle(element);
  const attrs = {};
  for (const a of element.attributes || []) {
    attrs[a.name] = sensitive(a.name) ? '[redacted]' : String(a.value).slice(0, 2048);
  }
  const redact = (el) => {
    for (const a of Array.from(el.attributes || [])) {
      if (sensitive(a.name)) { try { el.setAttribute(a.name, '[redacted]'); } catch (e) {} }
    }
    const tag = String(el.tagName || '').toUpperCase();
    const type = String((el.getAttribute && el.getAttribute('type')) || '').toLowerCase();
    if (tag === 'INPUT' && type === 'password') {
      try { el.setAttribute('value', '[redacted]'); } catch (e) {}
    }
  };
  const clone = element.cloneNode(true);
  redact(clone);
  if (clone.querySelectorAll) { for (const d of clone.querySelectorAll('*')) { redact(d); } }
  return {
    tagName: element.tagName,
    attrs: attrs,
    rect: { x: r.x, y: r.y, width: r.width, height: r.height },
    style: { display: s.display, visibility: s.visibility, position: s.position, color: s.color, backgroundColor: s.backgroundColor, fontSize: s.fontSize },
    outerHTML: String(clone.outerHTML || '').slice(0, 32768)
  };
}"#;

pub(crate) const PERFORMANCE_FUNCTION: &str = "() => ({navigation: performance.getEntriesByType('navigation').map(e=>({name:e.name,duration:e.duration,domContentLoadedEventEnd:e.domContentLoadedEventEnd,loadEventEnd:e.loadEventEnd})), resources: performance.getEntriesByType('resource').slice(-100).map(e=>({name:e.name,initiatorType:e.initiatorType,duration:e.duration,transferSize:e.transferSize}))})";

struct RunningPlaywright {
    client: RunningService<RoleClient, ()>,
    _job: JobHandle,
    engine: BrowserEngine,
    /// OS pid of the Playwright MCP child process, when the transport reported
    /// one. Retained for diagnostics and test-only crash simulation.
    child_pid: Option<u32>,
}

pub struct PlaywrightMcpDriver {
    runtime_root: PathBuf,
    running: Mutex<Option<RunningPlaywright>>,
    diagnostics: Arc<Mutex<VecDeque<String>>>,
}

impl PlaywrightMcpDriver {
    pub fn new(runtime_root: PathBuf) -> Self {
        Self {
            runtime_root,
            running: Mutex::new(None),
            diagnostics: Arc::new(Mutex::new(VecDeque::with_capacity(100))),
        }
    }

    pub fn runtime_root(&self) -> &Path {
        &self.runtime_root
    }

    pub async fn diagnostics_snapshot(&self) -> Vec<String> {
        self.diagnostics.lock().await.iter().cloned().collect()
    }

    /// Test-only: OS pid of the running Playwright MCP child process, if known.
    /// Integration tests use it to walk the browser process tree and to prove
    /// Background launches create no visible top-level windows.
    pub async fn test_child_pid(&self) -> Option<u32> {
        self.running.lock().await.as_ref().and_then(|r| r.child_pid)
    }

    /// Test-only: forcefully terminate the Playwright MCP child process tree
    /// WITHOUT clearing driver state, simulating an unexpected browser crash. The
    /// next tool call must then surface `BROWSER_PROCESS_LOST` instead of a
    /// graceful stop. Returns the terminated pid.
    pub async fn test_terminate_child(&self) -> Result<u32, CoreError> {
        let pid = self.test_child_pid().await.ok_or_else(|| {
            CoreError::new(
                ErrorCode::BrowserNotRunning,
                "browser child process pid is unknown",
            )
        })?;
        kill_process_tree(pid);
        Ok(pid)
    }

    pub(crate) fn is_generic_tool_allowed(tool: &str) -> bool {
        GENERIC_TOOL_ALLOWLIST.contains(&tool) && !FORBIDDEN_PROCESS_CODE_TOOLS.contains(&tool)
    }

    pub(crate) fn launch_config(spec: &BrowserLaunchSpec) -> Value {
        let mut launch_options = Map::new();
        launch_options.insert(
            "headless".into(),
            Value::Bool(spec.run_mode == BrowserRunMode::Background),
        );
        if matches!(
            spec.engine,
            BrowserEngine::BundledChromium | BrowserEngine::Chrome | BrowserEngine::Edge
        ) {
            let mut args = vec![Value::String(
                "--disable-blink-features=AutomationControlled".into(),
            )];
            if spec.run_mode == BrowserRunMode::VisibleDebug {
                args.push(Value::String("--auto-open-devtools-for-tabs".into()));
            }
            launch_options.insert("args".into(), Value::Array(args));
        }
        match spec.engine {
            BrowserEngine::Chrome => {
                launch_options.insert("channel".into(), Value::String("chrome".into()));
            }
            BrowserEngine::Edge => {
                launch_options.insert("channel".into(), Value::String("msedge".into()));
            }
            BrowserEngine::BundledChromium | BrowserEngine::Firefox => {}
        }

        let mut context_options = Map::new();
        context_options.insert("acceptDownloads".into(), Value::Bool(true));
        if let Some(e) = &spec.emulation {
            context_options.insert(
                "viewport".into(),
                json!({ "width": e.width, "height": e.height }),
            );
            context_options.insert("deviceScaleFactor".into(), json!(e.device_scale_factor));
            context_options.insert("isMobile".into(), json!(e.is_mobile));
            context_options.insert("hasTouch".into(), json!(e.has_touch));
            if let Some(ua) = &e.user_agent {
                context_options.insert("userAgent".into(), json!(ua));
            }
            if e.landscape {
                context_options.insert(
                    "screen".into(),
                    json!({ "width": e.width.max(e.height), "height": e.width.min(e.height) }),
                );
            }
        }

        let browser_name = match spec.engine {
            BrowserEngine::Firefox => "firefox",
            BrowserEngine::BundledChromium | BrowserEngine::Chrome | BrowserEngine::Edge => {
                "chromium"
            }
        };
        json!({
            "browser": {
                "browserName": browser_name,
                "userDataDir": spec.profile_dir,
                "launchOptions": launch_options,
                "contextOptions": context_options
            },
            "capabilities": ["devtools"],
            "outputDir": spec.artifact_dir,
            "outputMaxSize": 536870912u64,
            "imageResponses": "allow",
            "allowUnrestrictedFileAccess": false,
            "codegen": "none",
            "timeouts": {
                "action": spec.action_timeout_ms,
                "navigation": spec.navigation_timeout_ms,
                "expect": 5000,
                "settle": 500
            }
        })
    }

    /// Full upstream dependency set: every tool any Local Workspaces handler may
    /// forward, plus the fixed Chromium performance primitive. Validated at
    /// startup so a mismatched/incomplete bundled runtime fails fast instead of
    /// erroring mid-session. Must stay a superset of `GENERIC_TOOL_ALLOWLIST`.
    ///
    /// `browser_navigate_forward` and `browser_reload` are omitted because the
    /// pinned upstream runtime (0.0.79) marks them `skillOnly` and never exposes
    /// them over MCP; requiring them would block every browser launch.
    fn required_upstream_tools() -> &'static [&'static str] {
        &[
            "browser_navigate",
            "browser_navigate_back",
            "browser_snapshot",
            "browser_click",
            "browser_hover",
            "browser_drag",
            "browser_type",
            "browser_select_option",
            "browser_press_key",
            "browser_handle_dialog",
            "browser_file_upload",
            "browser_resize",
            "browser_take_screenshot",
            "browser_console_messages",
            "browser_network_requests",
            "browser_evaluate",
            "browser_tabs",
            "browser_wait_for",
            "browser_close",
            "browser_start_tracing",
            "browser_stop_tracing",
            "browser_run_code_unsafe",
        ]
    }

    fn runtime_paths(&self) -> Result<(PathBuf, PathBuf, PathBuf), CoreError> {
        let node = self.runtime_root.join("node").join("node.exe");
        let cli = self
            .runtime_root
            .join("node_modules")
            .join("@playwright")
            .join("mcp")
            .join("cli.js");
        let browsers = self.runtime_root.join("browsers");
        for path in [&node, &cli, &browsers] {
            if !path.exists() {
                return Err(CoreError::new(
                    ErrorCode::BrowserStartFailed,
                    "bundled browser runtime is incomplete",
                ));
            }
        }
        Ok((node, cli, browsers))
    }

    fn map_protocol_error(tool: &str, text: &str) -> CoreError {
        let lowered = text.to_ascii_lowercase();
        if (lowered.contains("ref") || lowered.contains("target"))
            && (lowered.contains("not found")
                || lowered.contains("snapshot")
                || lowered.contains("stale"))
        {
            return CoreError::new(
                ErrorCode::BrowserStaleRef,
                "browser element reference is stale; take a new snapshot",
            );
        }
        if lowered.contains("timeout") || lowered.contains("timed out") {
            let code = if tool.contains("navigate") {
                ErrorCode::BrowserNavigationTimeout
            } else {
                ErrorCode::BrowserActionTimeout
            };
            return CoreError::new(code, "browser operation timed out");
        }
        let message = if lowered.contains("closed")
            || lowered.contains("disconnected")
            || lowered.contains("process") && lowered.contains("exit")
        {
            "browser process was lost"
        } else {
            "browser operation failed"
        };
        let code = if message.contains("process") {
            ErrorCode::BrowserProcessLost
        } else {
            ErrorCode::BrowserProtocolError
        };
        CoreError::new(code, message)
    }

    fn call_result_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn raw_call_locked(
        client: &RunningService<RoleClient, ()>,
        tool: &'static str,
        arguments: Map<String, Value>,
    ) -> Result<CallToolResult, CoreError> {
        let result = client
            .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
            .await
            .map_err(|_| {
                CoreError::new(
                    ErrorCode::BrowserProcessLost,
                    "browser process connection was lost",
                )
            })?;
        if result.is_error.unwrap_or(false) {
            return Err(Self::map_protocol_error(
                tool,
                &Self::call_result_text(&result),
            ));
        }
        Ok(result)
    }
}

#[async_trait::async_trait]
impl BrowserDriver for PlaywrightMcpDriver {
    async fn start(&self, spec: BrowserLaunchSpec) -> Result<BrowserDriverStatus, CoreError> {
        self.stop().await?;
        std::fs::create_dir_all(&spec.profile_dir).map_err(CoreError::io)?;
        std::fs::create_dir_all(&spec.artifact_dir).map_err(CoreError::io)?;

        let (node, cli, browsers) = self.runtime_paths()?;
        let config_path = spec.artifact_dir.join("playwright-mcp-config.json");
        let config = Self::launch_config(&spec);
        let config_bytes = serde_json::to_vec_pretty(&config).map_err(|_| {
            CoreError::new(
                ErrorCode::BrowserStartFailed,
                "failed to serialize browser launch configuration",
            )
        })?;
        std::fs::write(&config_path, config_bytes).map_err(CoreError::io)?;

        let mut command = tokio::process::Command::new(node);
        command
            .arg(cli)
            .arg("--config")
            .arg(&config_path)
            .env("PLAYWRIGHT_BROWSERS_PATH", browsers)
            .current_dir(&spec.artifact_dir);
        #[cfg(windows)]
        if spec.run_mode == BrowserRunMode::Background {
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let (transport, stderr) = TokioChildProcess::builder(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| {
                CoreError::new(
                    ErrorCode::BrowserStartFailed,
                    "failed to start bundled Playwright MCP process",
                )
            })?;
        let pid = transport.id();
        let job = JobHandle::create().map_err(|_| {
            CoreError::new(
                ErrorCode::BrowserStartFailed,
                "failed to create browser process containment job",
            )
        })?;
        if let Some(pid) = pid {
            job.assign_pid(pid).map_err(|_| {
                CoreError::new(
                    ErrorCode::BrowserStartFailed,
                    "failed to contain browser process",
                )
            })?;
        }

        if let Some(stderr) = stderr {
            let diagnostics = self.diagnostics.clone();
            tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    // Browser stderr is local diagnostics only. Bound both line
                    // size and history to avoid retaining arbitrary page data.
                    let bounded: String = line.chars().take(1024).collect();
                    let mut queue = diagnostics.lock().await;
                    if queue.len() == 100 {
                        queue.pop_front();
                    }
                    queue.push_back(bounded);
                }
            });
        }

        let client: RunningService<RoleClient, ()> = ().serve(transport).await.map_err(|_| {
            CoreError::new(
                ErrorCode::BrowserStartFailed,
                "Playwright MCP initialization failed",
            )
        })?;
        let discovered: BTreeSet<String> = client
            .list_all_tools()
            .await
            .map_err(|_| {
                CoreError::new(
                    ErrorCode::BrowserStartFailed,
                    "Playwright MCP tool discovery failed",
                )
            })?
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        let missing: Vec<&str> = Self::required_upstream_tools()
            .iter()
            .copied()
            .filter(|name| !discovered.contains(*name))
            .collect();
        if !missing.is_empty() {
            return Err(CoreError::new(
                ErrorCode::BrowserStartFailed,
                format!(
                    "bundled Playwright MCP is missing required browser tools: {}",
                    missing.join(", ")
                ),
            ));
        }

        let status = BrowserDriverStatus {
            running: true,
            actual_engine: spec.engine,
            sidecar_version: Some(PLAYWRIGHT_MCP_VERSION.into()),
            browser_version: None,
            child_pid: pid,
        };
        *self.running.lock().await = Some(RunningPlaywright {
            client,
            _job: job,
            engine: spec.engine,
            child_pid: pid,
        });
        Ok(status)
    }

    async fn stop(&self) -> Result<(), CoreError> {
        // Dropping the RunningService closes stdio; dropping JobHandle then
        // terminates any remaining process tree. This is intentionally
        // fail-safe rather than trusting graceful browser shutdown alone.
        self.running.lock().await.take();
        Ok(())
    }

    async fn call(
        &self,
        tool: &'static str,
        arguments: Map<String, Value>,
    ) -> Result<CallToolResult, CoreError> {
        if !Self::is_generic_tool_allowed(tool) {
            return Err(CoreError::new(
                ErrorCode::BrowserProtocolError,
                "requested upstream browser operation is not allowed",
            ));
        }
        let guard = self.running.lock().await;
        let running = guard.as_ref().ok_or_else(|| {
            CoreError::new(ErrorCode::BrowserNotRunning, "browser is not running")
        })?;
        Self::raw_call_locked(&running.client, tool, arguments).await
    }

    fn launch_devtools_auto_open(&self, spec: &BrowserLaunchSpec) -> bool {
        // Derived from the exact launch config handed to upstream, so the
        // reported state is the launch-time truth rather than a hard-coded
        // assumption.
        let config = Self::launch_config(spec);
        let options = &config["browser"]["launchOptions"];
        let headed = options["headless"] == Value::Bool(false);
        let auto_open = options["args"]
            .as_array()
            .map(|args| {
                args.iter()
                    .any(|flag| flag == "--auto-open-devtools-for-tabs")
            })
            .unwrap_or(false);
        headed && auto_open
    }

    async fn chromium_performance_metrics(&self) -> Result<Value, CoreError> {
        let guard = self.running.lock().await;
        let running = guard.as_ref().ok_or_else(|| {
            CoreError::new(ErrorCode::BrowserNotRunning, "browser is not running")
        })?;
        if running.engine == BrowserEngine::Firefox {
            return Err(CoreError::new(
                ErrorCode::BrowserFeatureUnsupported,
                "Chromium DevTools performance metrics are not supported by Firefox",
            ));
        }
        let mut args = Map::new();
        args.insert(
            "code".into(),
            Value::String(CHROMIUM_PERFORMANCE_CODE.into()),
        );
        let result =
            Self::raw_call_locked(&running.client, "browser_run_code_unsafe", args).await?;
        let text = Self::call_result_text(&result);
        Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({ "text": text })))
    }

    async fn device_descriptor(&self, name: &str) -> Result<Value, CoreError> {
        let path = self.runtime_root.join("device-descriptors.json");
        let bytes = std::fs::read(&path).map_err(|_| {
            CoreError::new(
                ErrorCode::BrowserFeatureUnsupported,
                "bundled Playwright device descriptors are unavailable",
            )
        })?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
            CoreError::new(
                ErrorCode::BrowserArtifactFailed,
                "bundled Playwright device descriptor data is invalid",
            )
        })?;
        value.get(name).cloned().ok_or_else(|| {
            CoreError::new(
                ErrorCode::BrowserFeatureUnsupported,
                format!("Playwright device preset '{name}' was not found"),
            )
        })
    }
}

/// Test-only helper: best-effort kill of a process and its descendants. On
/// Windows `taskkill /T` terminates the whole tree (node MCP + browser child
/// processes); elsewhere a plain SIGKILL is used.
#[cfg(windows)]
fn kill_process_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

#[cfg(not(windows))]
fn kill_process_tree(pid: u32) {
    let _ = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .output();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::types::{DEFAULT_ACTION_TIMEOUT_MS, DEFAULT_NAVIGATION_TIMEOUT_MS};

    fn spec(engine: BrowserEngine, run_mode: BrowserRunMode) -> BrowserLaunchSpec {
        BrowserLaunchSpec {
            session_id: "s1".into(),
            engine,
            run_mode,
            profile_dir: PathBuf::from("C:/profile"),
            artifact_dir: PathBuf::from("C:/artifacts/s1"),
            action_timeout_ms: DEFAULT_ACTION_TIMEOUT_MS,
            navigation_timeout_ms: DEFAULT_NAVIGATION_TIMEOUT_MS,
            emulation: None,
        }
    }

    #[test]
    fn generic_allowlist_rejects_process_code_tools() {
        assert!(PlaywrightMcpDriver::is_generic_tool_allowed(
            "browser_navigate"
        ));
        assert!(PlaywrightMcpDriver::is_generic_tool_allowed(
            "browser_start_tracing"
        ));
        assert!(!PlaywrightMcpDriver::is_generic_tool_allowed(
            "browser_run_code"
        ));
        assert!(!PlaywrightMcpDriver::is_generic_tool_allowed(
            "browser_run_code_unsafe"
        ));
        assert!(!PlaywrightMcpDriver::is_generic_tool_allowed(
            "browser_cookie_list"
        ));
    }

    #[test]
    fn startup_dependency_set_covers_every_forwarded_tool() {
        let required: std::collections::BTreeSet<&str> =
            PlaywrightMcpDriver::required_upstream_tools()
                .iter()
                .copied()
                .collect();
        for tool in GENERIC_TOOL_ALLOWLIST {
            assert!(
                required.contains(tool),
                "required_upstream_tools is missing forwarded tool {tool}"
            );
        }
        assert!(
            required.contains("browser_run_code_unsafe"),
            "Chromium performance primitive must be validated at startup"
        );
    }

    #[test]
    fn dom_inspection_sanitizes_outerhtml_from_a_clone() {
        let f = DOM_INSPECT_FUNCTION;
        // Sanitization must happen on a clone, not the live element.
        assert!(
            f.contains("cloneNode(true)"),
            "must clone before serializing"
        );
        // Sensitive attributes/password values are redacted before serialization.
        assert!(f.contains("[redacted]"), "must redact sensitive values");
        assert!(
            f.contains("setAttribute"),
            "must overwrite sensitive attributes"
        );
        // The serialized markup must come from the clone, never the live element.
        assert!(f.contains("clone.outerHTML"), "must serialize the clone");
        assert!(
            !f.contains("element.outerHTML"),
            "must not serialize the live element's outerHTML directly"
        );
    }

    #[test]
    fn fixed_expressions_have_no_interpolation_points() {
        for f in [
            WAIT_VISIBLE_FUNCTION,
            WAIT_URL_FUNCTION,
            WAIT_READY_FUNCTION,
            DOM_INSPECT_FUNCTION,
            PERFORMANCE_FUNCTION,
            CHROMIUM_PERFORMANCE_CODE,
        ] {
            assert!(!f.contains("{0}"), "fixed expression has a format slot");
            assert!(!f.contains("${"), "fixed expression has a template slot");
        }
    }

    #[test]
    fn bundled_background_config_is_headless_sandboxed_and_devtools_capable() {
        let value = PlaywrightMcpDriver::launch_config(&spec(
            BrowserEngine::BundledChromium,
            BrowserRunMode::Background,
        ));
        assert_eq!(value["browser"]["browserName"], "chromium");
        assert_eq!(value["browser"]["launchOptions"]["headless"], true);
        assert_eq!(value["allowUnrestrictedFileAccess"], false);
        assert_eq!(value["capabilities"], json!(["devtools"]));
        assert_eq!(value["codegen"], "none");
        assert_eq!(value["timeouts"]["action"], 10_000);
        assert_eq!(value["timeouts"]["navigation"], 60_000);
    }

    #[test]
    fn visible_chromium_family_config_opens_devtools_without_affecting_firefox() {
        let chrome = PlaywrightMcpDriver::launch_config(&spec(
            BrowserEngine::Chrome,
            BrowserRunMode::VisibleDebug,
        ));
        assert_eq!(chrome["browser"]["launchOptions"]["channel"], "chrome");
        assert_eq!(chrome["browser"]["launchOptions"]["headless"], false);
        assert!(chrome["browser"]["launchOptions"]["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "--auto-open-devtools-for-tabs"));

        let firefox = PlaywrightMcpDriver::launch_config(&spec(
            BrowserEngine::Firefox,
            BrowserRunMode::VisibleDebug,
        ));
        assert_eq!(firefox["browser"]["browserName"], "firefox");
        assert!(firefox["browser"]["launchOptions"]["channel"].is_null());
    }

    #[test]
    fn devtools_auto_open_reports_launch_time_state_truthfully() {
        let driver = PlaywrightMcpDriver::new(PathBuf::from("runtime"));
        assert!(driver
            .launch_devtools_auto_open(&spec(BrowserEngine::Chrome, BrowserRunMode::VisibleDebug)));
        assert!(driver.launch_devtools_auto_open(&spec(
            BrowserEngine::BundledChromium,
            BrowserRunMode::VisibleDebug
        )));
        assert!(!driver
            .launch_devtools_auto_open(&spec(BrowserEngine::Chrome, BrowserRunMode::Background)));
        assert!(!driver.launch_devtools_auto_open(&spec(
            BrowserEngine::Firefox,
            BrowserRunMode::VisibleDebug
        )));
    }

    #[test]
    fn emulation_context_options_carry_full_device_spec() {
        let mut launch = spec(BrowserEngine::BundledChromium, BrowserRunMode::Background);
        launch.emulation = Some(super::super::types::BrowserEmulation {
            width: 390,
            height: 844,
            device_scale_factor: 3.0,
            is_mobile: true,
            has_touch: true,
            user_agent: Some("TestPhone/1.0".into()),
            landscape: false,
        });
        let config = PlaywrightMcpDriver::launch_config(&launch);
        let context = &config["browser"]["contextOptions"];
        assert_eq!(context["viewport"], json!({"width": 390, "height": 844}));
        assert_eq!(context["deviceScaleFactor"], 3.0);
        assert_eq!(context["isMobile"], true);
        assert_eq!(context["hasTouch"], true);
        assert_eq!(context["userAgent"], "TestPhone/1.0");
        assert_eq!(context["acceptDownloads"], true);
    }

    #[test]
    fn stale_ref_and_timeout_errors_map_to_stable_codes() {
        let stale = PlaywrightMcpDriver::map_protocol_error(
            "browser_click",
            "Ref e12 not found in the current page snapshot",
        );
        assert_eq!(stale.code, ErrorCode::BrowserStaleRef);

        let timeout = PlaywrightMcpDriver::map_protocol_error(
            "browser_navigate",
            "Timeout 30000ms exceeded while waiting for navigation",
        );
        assert_eq!(timeout.code, ErrorCode::BrowserNavigationTimeout);

        let action_timeout =
            PlaywrightMcpDriver::map_protocol_error("browser_click", "element timed out");
        assert_eq!(action_timeout.code, ErrorCode::BrowserActionTimeout);
    }

    #[test]
    fn upstream_paths_and_stack_frames_never_leak_into_error_messages() {
        let upstream = [
            "Error: ENOENT: no such file or directory",
            "C:\\Users\\x\\AppData\\node_modules\\@playwright\\mcp\\lib\\index.js:123",
            "    at Object.<anonymous> (C:\\secret\\path.js:1:1)",
        ]
        .join("\n");
        let error = PlaywrightMcpDriver::map_protocol_error("browser_click", &upstream);
        for forbidden in ["node_modules", "C:", "\\", "at Object", ".js"] {
            assert!(
                !error.message.contains(forbidden),
                "error message leaks {forbidden:?}: {}",
                error.message
            );
        }
    }
}
