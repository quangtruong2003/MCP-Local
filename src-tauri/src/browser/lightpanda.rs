//! Native Lightpanda MCP page driver.
//!
//! One logical page = one Lightpanda child stdio MCP process inside the
//! managed WSL2 distro, running as the unprivileged `localworkspaces` user.
//! The child is never exposed on a TCP port; this driver owns the only client
//! connection over child stdio and forwards a deliberately restricted tool
//! set. Admin tools (`getEnv`, `save`, `session_*`) are never callable;
//! `getCookies` stays internal-only (cookie snapshots) and is never exposed
//! as a public Local Workspaces tool.

use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lc_core::error::{CoreError, ErrorCode};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use serde_json::{Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::Mutex;

use crate::process_job::JobHandle;

use super::driver::{
    BrowserPageDriver, BrowserPageDriverFactory, PageDriverStatus, PageLaunchSpec,
};
use super::runtime::REQUIRED_NATIVE_TOOLS;
use super::wsl::build_lightpanda_command;

/// Fixed bound on HTTP cache entries handed to the child.
pub const HTTP_CACHE_ENTRY_LIMIT: u32 = 1_000;

/// Outer bound for one upstream call.
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Outer bound for tool discovery right after spawn. Unparsable child output
/// is silently dropped by the transport, so a misbehaving child could
/// otherwise hold discovery open forever; fail closed instead.
const DEFAULT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Navigation-class tools: timeouts on these map to the navigation timeout
/// code, everything else maps to the action timeout code.
const NAVIGATION_CLASS_TOOLS: &[&str] =
    &["goto", "waitForSelector", "waitForState", "waitForScript"];

/// One spawned page child plus its connected MCP client.
pub struct SpawnedPageChild {
    pub client: RunningService<RoleClient, ()>,
    pub child_pid: Option<u32>,
    pub stderr: Option<Box<dyn AsyncRead + Unpin + Send>>,
    pub(crate) job: Option<JobHandle>,
}

/// Seam over the page child spawn so protocol behavior is testable without a
/// Windows host or a real distro.
#[async_trait]
pub trait PageChildSpawner: Send + Sync {
    async fn spawn(&self, extra_args: &[String]) -> Result<SpawnedPageChild, CoreError>;
}

/// Production spawner: fixed `wsl.exe` command running Lightpanda MCP as the
/// unprivileged runtime user inside the owned distro. Never uses the HTTP MCP
/// transport.
pub struct WslPageChildSpawner {
    wsl_exe: PathBuf,
    distro_name: String,
}

impl WslPageChildSpawner {
    pub fn new(wsl_exe: PathBuf, distro_name: String) -> Self {
        Self {
            wsl_exe,
            distro_name,
        }
    }
}

#[async_trait]
impl PageChildSpawner for WslPageChildSpawner {
    async fn spawn(&self, extra_args: &[String]) -> Result<SpawnedPageChild, CoreError> {
        let std_cmd = build_lightpanda_command(&self.wsl_exe, &self.distro_name, extra_args)?;
        let command = tokio::process::Command::from(std_cmd);
        let (transport, stderr) = TokioChildProcess::builder(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| {
                CoreError::new(
                    ErrorCode::BrowserStartFailed,
                    "failed to start lightpanda page process",
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
        let client: RunningService<RoleClient, ()> = ().serve(transport).await.map_err(|_| {
            CoreError::new(
                ErrorCode::BrowserStartFailed,
                "lightpanda MCP initialization failed",
            )
        })?;
        Ok(SpawnedPageChild {
            client,
            child_pid: pid,
            stderr: stderr.map(|stream| Box::new(stream) as Box<dyn AsyncRead + Unpin + Send>),
            job: Some(job),
        })
    }
}

struct RunningPage {
    client: RunningService<RoleClient, ()>,
    _job: Option<JobHandle>,
    lost: bool,
}

pub struct LightpandaPageDriver {
    spawner: Arc<dyn PageChildSpawner>,
    running: Mutex<Option<RunningPage>>,
    diagnostics: Arc<Mutex<VecDeque<String>>>,
    call_timeout: Duration,
    discovery_timeout: Duration,
}

impl LightpandaPageDriver {
    pub fn new(spawner: Arc<dyn PageChildSpawner>) -> Self {
        Self {
            spawner,
            running: Mutex::new(None),
            diagnostics: Arc::new(Mutex::new(VecDeque::with_capacity(100))),
            call_timeout: DEFAULT_CALL_TIMEOUT,
            discovery_timeout: DEFAULT_DISCOVERY_TIMEOUT,
        }
    }

    /// Test-only: shrink the per-call timeout bound.
    #[cfg(test)]
    fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// Test-only: shrink the discovery timeout bound.
    #[cfg(test)]
    fn with_discovery_timeout(mut self, timeout: Duration) -> Self {
        self.discovery_timeout = timeout;
        self
    }

    /// Fixed child argv suffix for one page. Every value is app-generated from
    /// the launch spec; MCP/model input never reaches this argv.
    pub(crate) fn page_args(spec: &PageLaunchSpec) -> Vec<String> {
        vec![
            "--cookie".to_string(),
            spec.cookie_path.clone(),
            "--cookie-jar".to_string(),
            spec.cookie_jar_path.clone(),
            "--storage-engine".to_string(),
            "sqlite".to_string(),
            "--storage-sqlite-path".to_string(),
            spec.storage_sqlite_path.clone(),
            "--http-cache-dir".to_string(),
            spec.http_cache_dir.clone(),
            "--http-cache-entry-limit".to_string(),
            HTTP_CACHE_ENTRY_LIMIT.to_string(),
        ]
    }

    /// The only tools reachable through this driver: the pinned required
    /// native set. Admin tools are absent by construction; `getCookies` is
    /// callable internally but never exposed publicly (gated higher up).
    pub(crate) fn is_tool_callable(tool: &str) -> bool {
        REQUIRED_NATIVE_TOOLS.contains(&tool)
    }

    /// Local diagnostics only: bounded snapshot of child stderr lines. Never
    /// surfaced through public MCP errors.
    pub async fn diagnostics_snapshot(&self) -> Vec<String> {
        self.diagnostics.lock().await.iter().cloned().collect()
    }

    fn timeout_error(tool: &str) -> CoreError {
        if NAVIGATION_CLASS_TOOLS.contains(&tool) {
            CoreError::new(
                ErrorCode::BrowserNavigationTimeout,
                "browser navigation timed out",
            )
        } else {
            CoreError::new(ErrorCode::BrowserActionTimeout, "browser action timed out")
        }
    }

    /// Fixed-message error mapping: upstream text is classified but never
    /// included in the public error.
    fn map_call_error(tool: &str, text: &str) -> CoreError {
        let lowered = text.to_ascii_lowercase();
        if lowered.contains("timeout") || lowered.contains("timed out") {
            return Self::timeout_error(tool);
        }
        if lowered.contains("stale") {
            return CoreError::new(
                ErrorCode::BrowserStaleRef,
                "browser element reference is stale; take a new snapshot",
            );
        }
        if (lowered.contains("ref") || lowered.contains("element") || lowered.contains("node"))
            && lowered.contains("not found")
        {
            return CoreError::new(
                ErrorCode::BrowserTargetNotFound,
                "browser element was not found on the page",
            );
        }
        CoreError::new(ErrorCode::BrowserProtocolError, "browser operation failed")
    }

    fn call_result_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[async_trait]
impl BrowserPageDriver for LightpandaPageDriver {
    async fn start(&self, spec: PageLaunchSpec) -> Result<PageDriverStatus, CoreError> {
        self.stop().await?;
        let args = Self::page_args(&spec);
        let spawned = self.spawner.spawn(&args).await?;

        if let Some(stderr) = spawned.stderr {
            let diagnostics = self.diagnostics.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    // Child stderr is local diagnostics only. Bound both line
                    // size and history so arbitrary page data is not retained.
                    let bounded: String = line.chars().take(1024).collect();
                    let mut queue = diagnostics.lock().await;
                    if queue.len() == 100 {
                        queue.pop_front();
                    }
                    queue.push_back(bounded);
                }
            });
        }

        let tools = tokio::time::timeout(self.discovery_timeout, spawned.client.list_all_tools())
            .await
            .map_err(|_| {
                CoreError::new(
                    ErrorCode::BrowserStartFailed,
                    "lightpanda MCP tool discovery timed out",
                )
            })?
            .map_err(|_| {
                CoreError::new(
                    ErrorCode::BrowserStartFailed,
                    "lightpanda MCP tool discovery failed",
                )
            })?;
        let discovered: BTreeSet<String> = tools
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        let missing = REQUIRED_NATIVE_TOOLS
            .iter()
            .filter(|name| !discovered.contains(**name))
            .count();
        if missing > 0 {
            return Err(CoreError::new(
                ErrorCode::BrowserRuntimeVersionMismatch,
                format!("lightpanda runtime is missing {missing} required native tools"),
            ));
        }

        let status = PageDriverStatus {
            running: true,
            page_id: spec.page_id,
            child_pid: spawned.child_pid,
        };
        *self.running.lock().await = Some(RunningPage {
            client: spawned.client,
            _job: spawned.job,
            lost: false,
        });
        Ok(status)
    }

    async fn call(
        &self,
        tool: &'static str,
        arguments: Map<String, Value>,
    ) -> Result<CallToolResult, CoreError> {
        if !Self::is_tool_callable(tool) {
            return Err(CoreError::new(
                ErrorCode::BrowserProtocolError,
                "requested upstream browser operation is not allowed",
            ));
        }
        let mut guard = self.running.lock().await;
        let running = guard.as_mut().ok_or_else(|| {
            CoreError::new(ErrorCode::BrowserNotRunning, "browser page is not running")
        })?;
        if running.lost {
            return Err(CoreError::new(
                ErrorCode::BrowserProcessLost,
                "browser page process was lost",
            ));
        }
        let request = running
            .client
            .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments));
        match tokio::time::timeout(self.call_timeout, request).await {
            Err(_) => Err(Self::timeout_error(tool)),
            Ok(Err(rmcp::ServiceError::McpError(mcp))) => {
                Err(Self::map_call_error(tool, mcp.message.as_ref()))
            }
            Ok(Err(_)) => {
                running.lost = true;
                Err(CoreError::new(
                    ErrorCode::BrowserProcessLost,
                    "browser page process connection was lost",
                ))
            }
            Ok(Ok(result)) => {
                if result.is_error.unwrap_or(false) {
                    return Err(Self::map_call_error(tool, &Self::call_result_text(&result)));
                }
                Ok(result)
            }
        }
    }

    async fn stop(&self) -> Result<(), CoreError> {
        // Dropping the RunningService closes stdio; dropping the JobHandle
        // then terminates any remaining process tree. Fail-safe by design.
        self.running.lock().await.take();
        Ok(())
    }

    async fn is_alive(&self) -> bool {
        self.running
            .lock()
            .await
            .as_ref()
            .is_some_and(|page| !page.lost)
    }
}

/// Factory used by the browser manager to create one driver per page.
pub struct LightpandaPageDriverFactory {
    spawner: Arc<dyn PageChildSpawner>,
}

impl LightpandaPageDriverFactory {
    pub fn new(spawner: Arc<dyn PageChildSpawner>) -> Self {
        Self { spawner }
    }
}

impl BrowserPageDriverFactory for LightpandaPageDriverFactory {
    fn create(&self) -> Arc<dyn BrowserPageDriver> {
        Arc::new(LightpandaPageDriver::new(self.spawner.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::transport::IntoTransport;
    use serde_json::json;
    use tokio::io::{duplex, AsyncWriteExt, DuplexStream};

    #[derive(Clone, Debug)]
    enum FakeMode {
        /// Full required tool set + admin extras; calls succeed.
        Normal,
        /// Omits one required tool (waitForScript) from discovery.
        MissingTool,
        /// Answers tools/list with non-JSON garbage.
        MalformedDiscovery,
        /// Dies on the first tools/call (clean EOF).
        CrashOnFirstCall,
        /// tools/call sleeps before answering.
        DelayedCall(Duration),
        /// tools/call returns isError with sensitive-looking text.
        SensitiveError,
    }

    const ADMIN_EXTRAS: &[&str] = &[
        "getEnv",
        "save",
        "session_new",
        "session_list",
        "session_close",
    ];

    fn fake_tool_list(missing_required: bool) -> Vec<Value> {
        let mut tools: Vec<Value> = REQUIRED_NATIVE_TOOLS
            .iter()
            .filter(|name| !(missing_required && **name == "waitForScript"))
            .map(|name| json!({ "name": name, "inputSchema": { "type": "object" } }))
            .collect();
        for name in ADMIN_EXTRAS {
            tools.push(json!({ "name": name, "inputSchema": { "type": "object" } }));
        }
        tools
    }

    /// In-process fake native MCP child speaking newline-delimited JSON-RPC.
    async fn run_fake_server(io: DuplexStream, mode: FakeMode) {
        let (read, mut write) = tokio::io::split(io);
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let Some(id) = value.get("id").cloned() else {
                continue; // notification
            };
            let method = value
                .get("method")
                .and_then(|m| m.as_str())
                .unwrap_or_default();
            let response = match method {
                "initialize" => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2025-03-26",
                        "capabilities": {},
                        "serverInfo": { "name": "fake-lightpanda", "version": "0.3.7" }
                    }
                })),
                "tools/list" => {
                    if matches!(mode, FakeMode::MalformedDiscovery) {
                        let _ = write.write_all(b"this is not json-rpc\n").await;
                        None
                    } else {
                        Some(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "tools": fake_tool_list(matches!(mode, FakeMode::MissingTool))
                            }
                        }))
                    }
                }
                "tools/call" => {
                    if let FakeMode::DelayedCall(delay) = mode {
                        tokio::time::sleep(delay).await;
                    }
                    if matches!(mode, FakeMode::CrashOnFirstCall) {
                        return; // clean EOF mid-call
                    }
                    let sensitive = matches!(mode, FakeMode::SensitiveError);
                    let text = if sensitive {
                        "ref e3 not found (cookie=/var/lib/local-workspaces/browser/cookies/page.json, token=secret-abc)"
                    } else {
                        "ok"
                    };
                    Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": text }],
                            "isError": sensitive
                        }
                    }))
                }
                _ => Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
            };
            let Some(response) = response else {
                continue;
            };
            let mut out = serde_json::to_string(&response).unwrap();
            out.push('\n');
            if write.write_all(out.as_bytes()).await.is_err() {
                return;
            }
        }
    }

    struct FakeSpawner {
        mode: FakeMode,
        stderr_lines: Vec<String>,
        spawned_args: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl FakeSpawner {
        fn new(mode: FakeMode) -> Self {
            Self {
                mode,
                stderr_lines: Vec::new(),
                spawned_args: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_stderr(mut self, lines: Vec<String>) -> Self {
            self.stderr_lines = lines;
            self
        }
    }

    #[async_trait]
    impl PageChildSpawner for FakeSpawner {
        async fn spawn(&self, extra_args: &[String]) -> Result<SpawnedPageChild, CoreError> {
            self.spawned_args.lock().await.push(extra_args.to_vec());
            let (client_io, server_io) = duplex(1 << 16);
            let mode = self.mode.clone();
            tokio::spawn(run_fake_server(server_io, mode));

            let stderr: Option<Box<dyn AsyncRead + Unpin + Send>> = if self.stderr_lines.is_empty()
            {
                None
            } else {
                let (read, mut write) = duplex(1 << 14);
                let lines = self.stderr_lines.clone();
                tokio::spawn(async move {
                    for line in lines {
                        if write
                            .write_all(format!("{line}\n").as_bytes())
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    let _ = write.flush().await;
                });
                Some(Box::new(read))
            };

            let transport = client_io.into_transport();
            let client: RunningService<RoleClient, ()> = ().serve(transport).await.map_err(|_| {
                CoreError::new(
                    ErrorCode::BrowserStartFailed,
                    "fake child initialization failed",
                )
            })?;
            Ok(SpawnedPageChild {
                client,
                child_pid: Some(4242),
                stderr,
                job: None,
            })
        }
    }

    fn test_spec() -> PageLaunchSpec {
        PageLaunchSpec {
            page_id: "page-1".into(),
            cookie_path: "/var/lib/local-workspaces/browser/cookies/canonical.json".into(),
            cookie_jar_path: "/var/lib/local-workspaces/browser/cookies/page-1.exit.json".into(),
            storage_sqlite_path: "/var/lib/local-workspaces/browser/profile/storage.sqlite".into(),
            http_cache_dir: "/var/lib/local-workspaces/browser/cache".into(),
        }
    }

    type DriverAndSpawnedArgs = (Arc<LightpandaPageDriver>, Arc<Mutex<Vec<Vec<String>>>>);

    fn driver_for(mode: FakeMode) -> DriverAndSpawnedArgs {
        let spawner = FakeSpawner::new(mode);
        let args = spawner.spawned_args.clone();
        (Arc::new(LightpandaPageDriver::new(Arc::new(spawner))), args)
    }

    #[tokio::test]
    async fn start_discovers_required_tools_and_calls_succeed() {
        let (driver, spawned_args) = driver_for(FakeMode::Normal);
        let status = driver.start(test_spec()).await.unwrap();
        assert!(status.running);
        assert_eq!(status.page_id, "page-1");
        assert_eq!(status.child_pid, Some(4242));
        assert!(driver.is_alive().await);

        // The child argv is fixed and spec-driven only.
        let args = spawned_args.lock().await.clone();
        assert_eq!(args.len(), 1);
        let argv = &args[0];
        let spec = test_spec();
        for expected in [
            "--cookie".to_string(),
            spec.cookie_path,
            "--cookie-jar".to_string(),
            spec.cookie_jar_path,
            "--storage-engine".to_string(),
            "sqlite".to_string(),
            "--storage-sqlite-path".to_string(),
            spec.storage_sqlite_path,
            "--http-cache-dir".to_string(),
            spec.http_cache_dir,
            "--http-cache-entry-limit".to_string(),
            HTTP_CACHE_ENTRY_LIMIT.to_string(),
        ] {
            assert!(argv.contains(&expected), "argv missing {expected}");
        }

        let result = driver.call("goto", Map::new()).await.unwrap();
        assert!(!result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn start_fails_closed_when_a_required_tool_is_missing() {
        let (driver, _) = driver_for(FakeMode::MissingTool);
        let err = driver.start(test_spec()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserRuntimeVersionMismatch);
        assert!(!driver.is_alive().await);
    }

    #[tokio::test]
    async fn admin_tools_are_never_callable() {
        let (driver, _) = driver_for(FakeMode::Normal);
        driver.start(test_spec()).await.unwrap();
        for tool in [
            "getEnv",
            "save",
            "session_new",
            "session_list",
            "session_close",
        ] {
            let err = driver.call(tool, Map::new()).await.unwrap_err();
            assert_eq!(
                err.code,
                ErrorCode::BrowserProtocolError,
                "tool {tool} must be rejected"
            );
        }
        // getCookies is internal-callable (cookie snapshots); public exposure
        // is gated at the tool layer, not here.
        assert!(driver.call("getCookies", Map::new()).await.is_ok());
    }

    #[tokio::test]
    async fn malformed_protocol_fails_start_closed() {
        // The transport silently drops unparsable lines, so a garbage
        // tools/list answer never errors on its own; discovery must hit its
        // bounded timeout and fail closed instead of hanging.
        let spawner = FakeSpawner::new(FakeMode::MalformedDiscovery);
        let driver = LightpandaPageDriver::new(Arc::new(spawner))
            .with_discovery_timeout(Duration::from_millis(200));
        let err = driver.start(test_spec()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserStartFailed);
        assert!(!driver.is_alive().await);
    }

    #[tokio::test]
    async fn child_crash_maps_to_process_lost() {
        let (driver, _) = driver_for(FakeMode::CrashOnFirstCall);
        driver.start(test_spec()).await.unwrap();
        let err = driver.call("goto", Map::new()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserProcessLost);
        assert!(!driver.is_alive().await);
        // Subsequent calls fail fast on the latched lost flag.
        let err = driver.call("getUrl", Map::new()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserProcessLost);
    }

    #[tokio::test]
    async fn call_timeouts_map_to_navigation_and_action_codes() {
        let spawner = FakeSpawner::new(FakeMode::DelayedCall(Duration::from_millis(400)));
        let driver = LightpandaPageDriver::new(Arc::new(spawner))
            .with_call_timeout(Duration::from_millis(100));
        driver.start(test_spec()).await.unwrap();
        let err = driver.call("goto", Map::new()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserNavigationTimeout);
        let err = driver.call("click", Map::new()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserActionTimeout);
    }

    #[tokio::test]
    async fn upstream_error_text_never_leaks_into_public_errors() {
        let (driver, _) = driver_for(FakeMode::SensitiveError);
        driver.start(test_spec()).await.unwrap();
        let err = driver.call("click", Map::new()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserTargetNotFound);
        let lowered = err.message.to_lowercase();
        for forbidden in [
            "/var/lib",
            "cookie=",
            "token",
            "secret",
            "page.json",
            "ref e3",
        ] {
            assert!(
                !lowered.contains(forbidden),
                "public error leaks {forbidden:?}: {}",
                err.message
            );
        }
    }

    #[tokio::test]
    async fn stderr_diagnostics_are_captured_locally_only() {
        let spawner = FakeSpawner::new(FakeMode::Normal).with_stderr(vec![
            "cookie file /var/lib/local-workspaces/browser/cookies/page-1.json loaded".into(),
        ]);
        let driver = LightpandaPageDriver::new(Arc::new(spawner));
        driver.start(test_spec()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let snapshot = driver.diagnostics_snapshot().await;
        assert!(
            snapshot.iter().any(|line| line.contains("cookie file")),
            "expected local stderr diagnostics, got {snapshot:?}"
        );
    }

    #[tokio::test]
    async fn stop_ends_the_page_and_is_alive_reports_false() {
        let (driver, _) = driver_for(FakeMode::Normal);
        driver.start(test_spec()).await.unwrap();
        assert!(driver.is_alive().await);
        driver.stop().await.unwrap();
        assert!(!driver.is_alive().await);
        let err = driver.call("goto", Map::new()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserNotRunning);
    }

    #[tokio::test]
    async fn start_replaces_a_running_page() {
        let (driver, spawned_args) = driver_for(FakeMode::Normal);
        driver.start(test_spec()).await.unwrap();
        let mut second = test_spec();
        second.page_id = "page-2".into();
        let status = driver.start(second).await.unwrap();
        assert_eq!(status.page_id, "page-2");
        assert_eq!(spawned_args.lock().await.len(), 2);
    }

    #[test]
    fn page_args_are_fixed_and_spec_driven() {
        let spec = test_spec();
        let expected: Vec<String> = vec![
            "--cookie".into(),
            spec.cookie_path.clone(),
            "--cookie-jar".into(),
            spec.cookie_jar_path.clone(),
            "--storage-engine".into(),
            "sqlite".into(),
            "--storage-sqlite-path".into(),
            spec.storage_sqlite_path.clone(),
            "--http-cache-dir".into(),
            spec.http_cache_dir.clone(),
            "--http-cache-entry-limit".into(),
            HTTP_CACHE_ENTRY_LIMIT.to_string(),
        ];
        assert_eq!(LightpandaPageDriver::page_args(&spec), expected);
    }

    #[test]
    fn callable_set_is_exactly_the_required_native_tools() {
        for tool in REQUIRED_NATIVE_TOOLS {
            assert!(
                LightpandaPageDriver::is_tool_callable(tool),
                "{tool} must be callable"
            );
        }
        for tool in [
            "getEnv",
            "save",
            "session_new",
            "session_list",
            "session_close",
            "anything_else",
        ] {
            assert!(
                !LightpandaPageDriver::is_tool_callable(tool),
                "{tool} must be rejected"
            );
        }
    }
}
