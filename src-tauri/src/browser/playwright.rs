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

use crate::{
    process_job::JobHandle,
    settings::{BrowserEngine, BrowserRunMode},
};

use super::{
    driver::{BrowserDriver, BrowserDriverStatus},
    types::BrowserLaunchSpec,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PLAYWRIGHT_MCP_VERSION: &str = "0.0.79";

/// RCE-equivalent upstream tools must never be reachable via generic proxying.
const FORBIDDEN_PROCESS_CODE_TOOLS: &[&str] = &["browser_run_code", "browser_run_code_unsafe"];

/// Upstream methods Local Workspaces intentionally composes. Storage/cookies,
/// routes, arbitrary network bodies, coordinate input, and unsafe code are not
/// exposed by this adapter.
pub const GENERIC_TOOL_ALLOWLIST: &[&str] = &[
    "browser_navigate",
    "browser_navigate_back",
    "browser_navigate_forward",
    "browser_reload",
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

struct RunningPlaywright {
    client: RunningService<RoleClient, ()>,
    _job: JobHandle,
    engine: BrowserEngine,
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

    fn required_upstream_tools() -> &'static [&'static str] {
        &[
            "browser_navigate",
            "browser_snapshot",
            "browser_click",
            "browser_type",
            "browser_tabs",
            "browser_evaluate",
            "browser_take_screenshot",
            "browser_console_messages",
            "browser_network_requests",
            "browser_start_tracing",
            "browser_stop_tracing",
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
}
