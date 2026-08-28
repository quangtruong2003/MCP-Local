//! Native Lightpanda MCP page driver.
//!
//! One logical page = one Lightpanda child stdio MCP process inside the
//! managed WSL2 distro, running as the unprivileged `localworkspaces` user.
//! The child is never exposed on a TCP port; this driver owns the only client
//! connection over child stdio and forwards a deliberately restricted tool
//! set. Admin tools (`getEnv`, `save`, `session_*`) are never callable;
//! `getCookies` stays internal-only (cookie snapshots) and is never exposed
//! as a public Local Workspaces tool.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
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
use super::wsl::build_lightpanda_command;

/// Interim restricted page-mode tool set: the pinned native contract minus
/// the admin/session tools. Task 4 replaces this module with the native HTTP
/// service, which uses the full 31-tool contract directly.
const NATIVE_SESSION_ADMIN_TOOLS: &[&str] = &[
    "getEnv",
    "save",
    "session_new",
    "session_list",
    "session_close",
];

fn required_native_tools() -> Vec<&'static str> {
    super::contract::NATIVE_TOOL_MAPPINGS
        .iter()
        .map(|(native, _)| *native)
        .filter(|name| !NATIVE_SESSION_ADMIN_TOOLS.contains(name))
        .collect()
}

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
    /// Local diagnostics only; never surfaced through MCP handlers.
    child_pid: Option<u32>,
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
    ///
    /// The pinned Lightpanda 0.3.7 `mcp` mode rejects the `--storage-engine`
    /// and `--storage-sqlite-path` flags (verified against the pinned binary:
    /// `unknown argument`), so Web Storage is per-page and volatile. Only
    /// flags the pinned binary accepts may appear here.
    pub(crate) fn page_args(spec: &PageLaunchSpec) -> Vec<String> {
        vec![
            "--cookie".to_string(),
            spec.cookie_path.clone(),
            "--cookie-jar".to_string(),
            spec.cookie_jar_path.clone(),
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
        required_native_tools().contains(&tool)
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
        let required = required_native_tools();
        let missing = required
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
            child_pid: spawned.child_pid,
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

    fn child_pid(&self) -> Option<u32> {
        self.running
            .try_lock()
            .ok()
            .and_then(|guard| guard.as_ref().and_then(|page| page.child_pid))
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

/// Production spawner that resolves the owned distro from the runtime
/// provisioner's persisted identity at spawn time. The runtime gate has
/// already ensured readiness before any page spawn, so a missing identity
/// here is a fail-closed invariant violation, not a provisioning trigger.
pub struct RuntimeResolvedSpawner {
    runtime: Arc<crate::browser::runtime::BrowserRuntimeManager>,
    wsl_exe: PathBuf,
}

impl RuntimeResolvedSpawner {
    pub fn new(runtime: Arc<crate::browser::runtime::BrowserRuntimeManager>) -> Self {
        Self {
            runtime,
            wsl_exe: PathBuf::from("wsl.exe"),
        }
    }
}

#[async_trait]
impl PageChildSpawner for RuntimeResolvedSpawner {
    async fn spawn(&self, extra_args: &[String]) -> Result<SpawnedPageChild, CoreError> {
        let identity = self.runtime.distro_identity().ok_or_else(|| {
            CoreError::new(
                ErrorCode::BrowserRuntimeNotReady,
                "browser runtime is not provisioned yet",
            )
        })?;
        WslPageChildSpawner::new(self.wsl_exe.clone(), identity.distro_name)
            .spawn(extra_args)
            .await
    }
}

// ---------------------------------------------------------------------------
// Native HTTP service: one shared Lightpanda server, native sessions
// ---------------------------------------------------------------------------

/// Tools that manage native sessions. They are serialized behind one global
/// mutex and always route over the `default` bootstrap session; the session
/// they operate on is a native tool argument, never the routing header.
const SESSION_CONTROL_TOOLS: &[&str] = &["session_new", "session_list", "session_close"];

/// Server-facing seam: one native `tools/call` with the exact upstream
/// arguments and result. Lifecycle is intentionally not exposed to MCP
/// handlers.
#[async_trait]
pub trait BrowserNativeCaller: Send + Sync {
    async fn call_native(
        &self,
        tool: &'static str,
        arguments: Map<String, Value>,
        session_id: Option<&str>,
    ) -> Result<super::http::UpstreamToolResponse, CoreError>;
}

/// Runtime gate seam: ensures the managed runtime is provisioned and returns
/// the owned distro name.
#[async_trait]
pub trait NativeRuntimeGate: Send + Sync {
    async fn ensure_ready(&self) -> Result<String, CoreError>;
    /// Best-effort in-distro stop (`pkill -TERM -x lightpanda`).
    async fn stop_server(&self, distro_name: &str) -> Result<(), CoreError>;
}

/// One spawned Lightpanda HTTP server child owned by the service.
pub struct SpawnedNativeServer {
    /// `None` only in test fixtures without a real child process.
    pub child: Option<tokio::process::Child>,
    pub(crate) job: Option<JobHandle>,
}

/// Launcher seam: spawns the shared server on the app-selected loopback port.
#[async_trait]
pub trait NativeServerLauncher: Send + Sync {
    async fn spawn(&self, distro_name: &str, port: u16) -> Result<SpawnedNativeServer, CoreError>;
}

/// Production runtime gate backed by the managed WSL provisioner.
pub struct NativeRuntimeManagerGate {
    runtime: Arc<super::runtime::BrowserRuntimeManager>,
}

impl NativeRuntimeManagerGate {
    pub fn new(runtime: Arc<super::runtime::BrowserRuntimeManager>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl NativeRuntimeGate for NativeRuntimeManagerGate {
    async fn ensure_ready(&self) -> Result<String, CoreError> {
        self.runtime.ensure_runtime().await?;
        let identity = self.runtime.distro_identity().ok_or_else(|| {
            CoreError::new(
                ErrorCode::BrowserRuntimeNotReady,
                "browser runtime is not provisioned yet",
            )
        })?;
        Ok(identity.distro_name)
    }

    async fn stop_server(&self, distro_name: &str) -> Result<(), CoreError> {
        self.runtime
            .wsl_adapter()
            .stop_lightpanda_server(distro_name)
            .await
    }
}

/// Production launcher: fixed WSL argv from `build_lightpanda_http_command`,
/// contained in a Windows job object, killed on drop. A child that cannot be
/// contained is killed immediately and never used.
pub struct WslNativeServerLauncher {
    wsl_exe: PathBuf,
}

impl WslNativeServerLauncher {
    pub fn new(wsl_exe: PathBuf) -> Self {
        Self { wsl_exe }
    }
}

#[async_trait]
impl NativeServerLauncher for WslNativeServerLauncher {
    async fn spawn(&self, distro_name: &str, port: u16) -> Result<SpawnedNativeServer, CoreError> {
        let std_cmd = super::wsl::build_lightpanda_http_command(&self.wsl_exe, distro_name, port)?;
        let mut child = tokio::process::Command::from(std_cmd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                CoreError::new(
                    ErrorCode::BrowserWslUnavailable,
                    format!("failed to spawn lightpanda http server: {e}"),
                )
            })?;
        let job = match JobHandle::create() {
            Ok(job) => job,
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(CoreError::new(
                    ErrorCode::BrowserStartFailed,
                    "failed to create browser process containment job",
                ));
            }
        };
        if let Some(pid) = child.id() {
            if job.assign_pid(pid).is_err() {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(CoreError::new(
                    ErrorCode::BrowserStartFailed,
                    "failed to contain lightpanda http server",
                ));
            }
        }
        Ok(SpawnedNativeServer {
            child: Some(child),
            job: Some(job),
        })
    }
}

/// One verified running server instance.
struct NativeInstance {
    distro_name: String,
    child: Option<tokio::process::Child>,
    _job: Option<JobHandle>,
    client: Arc<super::http::LightpandaHttpClient>,
    native_tool_count: usize,
}

/// Public status snapshot of the native service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeServiceStatus {
    pub enabled: bool,
    pub running: bool,
    pub contract_verified: bool,
    pub native_tool_count: usize,
}

/// Single shared native Lightpanda service: one lazily started private
/// loopback HTTP server, native sessions, full-contract verification at
/// startup. No page/tab concept exists here; the native session tools are
/// the only isolation primitive.
pub struct LightpandaNativeService {
    runtime: Arc<dyn NativeRuntimeGate>,
    launcher: Arc<dyn NativeServerLauncher>,
    enabled: AtomicBool,
    paused: AtomicBool,
    /// Single-flight startup serialization.
    starting: tokio::sync::Mutex<()>,
    instance: tokio::sync::Mutex<Option<NativeInstance>>,
    /// Weak keyed per-session locks: the same session serializes mutations,
    /// different sessions run concurrently, and arbitrary failed session ids
    /// do not permanently grow the map.
    session_locks: std::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
    /// Session-control calls are serialized globally and route on default.
    session_control: tokio::sync::Mutex<()>,
}

impl LightpandaNativeService {
    pub fn new(
        runtime: Arc<dyn NativeRuntimeGate>,
        launcher: Arc<dyn NativeServerLauncher>,
        settings: super::types::BrowserRuntimeSettings,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            launcher,
            enabled: AtomicBool::new(settings.enabled),
            paused: AtomicBool::new(false),
            starting: tokio::sync::Mutex::new(()),
            instance: tokio::sync::Mutex::new(None),
            session_locks: std::sync::Mutex::new(HashMap::new()),
            session_control: tokio::sync::Mutex::new(()),
        })
    }

    pub async fn status(&self) -> NativeServiceStatus {
        let instance = self.instance.lock().await;
        let running = instance.is_some();
        NativeServiceStatus {
            enabled: self.enabled.load(Ordering::SeqCst),
            running,
            // An instance is only ever published after full contract
            // verification, so running implies verified.
            contract_verified: running,
            native_tool_count: instance.as_ref().map(|i| i.native_tool_count).unwrap_or(0),
        }
    }

    /// Applies new settings. Disabling stops a running server.
    pub async fn apply_settings(&self, settings: super::types::BrowserRuntimeSettings) {
        let was_enabled = self.enabled.swap(settings.enabled, Ordering::SeqCst);
        if was_enabled && !settings.enabled {
            self.stop().await;
        }
    }

    /// Pause is authoritative: pausing stops a running server, and calls
    /// while paused fail closed.
    pub async fn on_pause_changed(&self, paused: bool) {
        let was_paused = self.paused.swap(paused, Ordering::SeqCst);
        if paused && !was_paused {
            self.stop().await;
        }
    }

    /// Stops the running server: bounded graceful in-distro stop first, then
    /// process/job kill containment. Safe to call when nothing is running.
    pub async fn stop(&self) {
        let Some(mut instance) = self.instance.lock().await.take() else {
            return;
        };
        let _ = self.runtime.stop_server(&instance.distro_name).await;
        if let Some(child) = instance.child.as_mut() {
            tokio::select! {
                _ = child.wait() => {}
                _ = tokio::time::sleep(Duration::from_secs(3)) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                }
            }
        }
        // Dropping the JobHandle completes containment cleanup.
    }

    /// Returns a live verified client, starting the shared server lazily.
    /// Startup is single-flight; a lost instance is replaced only by the
    /// next independent call, never replayed automatically.
    async fn ensure_running(&self) -> Result<Arc<super::http::LightpandaHttpClient>, CoreError> {
        if let Some(instance) = self.instance.lock().await.as_ref() {
            return Ok(instance.client.clone());
        }
        let _single_flight = self.starting.lock().await;
        if let Some(instance) = self.instance.lock().await.as_ref() {
            return Ok(instance.client.clone());
        }

        let distro_name = self.runtime.ensure_ready().await?;
        let port = super::runtime::pick_loopback_port()?;
        let spawned = self.launcher.spawn(&distro_name, port).await?;
        let startup = async {
            super::runtime::wait_for_loopback(port).await?;
            let endpoint: url::Url =
                format!("http://127.0.0.1:{port}/mcp")
                    .parse()
                    .map_err(|e| {
                        CoreError::new(
                            ErrorCode::BrowserProtocolError,
                            format!("invalid lightpanda endpoint: {e}"),
                        )
                    })?;
            let client = super::http::LightpandaHttpClient::new(endpoint, DEFAULT_CALL_TIMEOUT)?;
            super::runtime::verify_http_contract(&client).await?;
            Ok::<_, CoreError>(client)
        }
        .await;
        match startup {
            Ok(client) => {
                let client = Arc::new(client);
                *self.instance.lock().await = Some(NativeInstance {
                    distro_name,
                    child: spawned.child,
                    _job: spawned.job,
                    client: client.clone(),
                    // Contract verification just proved exact 31-tool parity.
                    native_tool_count: super::contract::EXPECTED_NATIVE_TOOL_COUNT,
                });
                Ok(client)
            }
            Err(error) => {
                // Never leave a half-started server behind.
                let mut child = spawned.child;
                if let Some(child) = child.as_mut() {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                }
                Err(error)
            }
        }
    }

    fn lock_for_session(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self.session_locks.lock().unwrap();
        map.retain(|_, weak| weak.strong_count() > 0);
        if let Some(lock) = map.get(id).and_then(std::sync::Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        map.insert(id.to_string(), Arc::downgrade(&lock));
        lock
    }

    /// Drops an unhealthy instance after connection loss so the next
    /// independent call may start a fresh server.
    async fn drop_unhealthy(&self) {
        let Some(mut instance) = self.instance.lock().await.take() else {
            return;
        };
        if let Some(child) = instance.child.as_mut() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }
}

#[async_trait]
impl BrowserNativeCaller for LightpandaNativeService {
    async fn call_native(
        &self,
        tool: &'static str,
        arguments: Map<String, Value>,
        session_id: Option<&str>,
    ) -> Result<super::http::UpstreamToolResponse, CoreError> {
        if !self.enabled.load(Ordering::SeqCst) || self.paused.load(Ordering::SeqCst) {
            return Err(CoreError::new(
                ErrorCode::BrowserAutomationDisabled,
                "Browser Automation is disabled",
            ));
        }
        if SESSION_CONTROL_TOOLS.contains(&tool) {
            let _serialization = self.session_control.lock().await;
            let client = self.ensure_running().await?;
            return client.call_tool(tool, arguments, None).await;
        }
        let effective = match session_id {
            Some(id) => {
                super::http::validate_session_id(id)?;
                id.to_string()
            }
            None => super::http::DEFAULT_SESSION_ID.to_string(),
        };
        let lock = self.lock_for_session(&effective);
        let _serialization = lock.lock().await;
        let client = self.ensure_running().await?;
        let outcome = client.call_tool(tool, arguments, session_id).await;
        if matches!(&outcome, Err(error) if error.code == ErrorCode::BrowserProcessLost) {
            self.drop_unhealthy().await;
        }
        outcome
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
        let mut tools: Vec<Value> = required_native_tools()
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
            "--http-cache-dir".into(),
            spec.http_cache_dir.clone(),
            "--http-cache-entry-limit".into(),
            HTTP_CACHE_ENTRY_LIMIT.to_string(),
        ];
        assert_eq!(LightpandaPageDriver::page_args(&spec), expected);
    }

    #[test]
    fn callable_set_is_exactly_the_required_native_tools() {
        for tool in required_native_tools() {
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

    // --- native service tests ------------------------------------------------

    use std::sync::atomic::AtomicUsize;

    #[derive(Clone)]
    struct NativeFakeServer {
        shutdown: Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        recorded: Arc<tokio::sync::Mutex<Vec<(String, Option<String>)>>>,
        in_flight: Arc<AtomicUsize>,
        max_in_flight: Arc<AtomicUsize>,
        call_delay: Duration,
    }

    impl NativeFakeServer {
        /// Stops the fake server entirely: further requests fail at the TCP
        /// level, exactly like a lost Lightpanda process.
        async fn kill(&self) {
            if let Some(tx) = self.shutdown.lock().await.take() {
                let _ = tx.send(());
            }
            // Give the listener a moment to close before assertions run.
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn native_pinned_tools_json() -> Value {
        let contract = crate::browser::contract::pinned_contract().unwrap();
        let tools: Vec<Value> = contract
            .tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                })
            })
            .collect();
        Value::Array(tools)
    }

    async fn native_fake_handler(
        axum::extract::State(server): axum::extract::State<NativeFakeServer>,
        headers: axum::http::HeaderMap,
        body: axum::body::Bytes,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        let body_value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let method = body_value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if method == "notifications/initialized" {
            return axum::http::StatusCode::ACCEPTED.into_response();
        }
        let id = body_value.get("id").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "lightpanda", "version": "0.1.0"},
            }),
            "tools/list" => serde_json::json!({ "tools": native_pinned_tools_json() }),
            "tools/call" => {
                let session = headers
                    .get("mcp-session-id")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let tool = body_value
                    .pointer("/params/name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let current = server.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                let mut max = server.max_in_flight.load(Ordering::SeqCst);
                while current > max {
                    match server.max_in_flight.compare_exchange(
                        max,
                        current,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => break,
                        Err(previous) => max = previous,
                    }
                }
                tokio::time::sleep(server.call_delay).await;
                server.in_flight.fetch_sub(1, Ordering::SeqCst);
                server.recorded.lock().await.push((tool.clone(), session));
                let text = if tool == "session_list" {
                    "[{\"id\":\"default\"}]".to_string()
                } else {
                    "ok".to_string()
                };
                serde_json::json!({
                    "content": [{"type": "text", "text": text}],
                    "isError": false,
                })
            }
            _ => serde_json::json!({}),
        };
        axum::Json(serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}))
            .into_response()
    }

    struct FakeNativeLauncher {
        launches: Arc<AtomicUsize>,
        call_delay: Duration,
        current: tokio::sync::Mutex<Option<NativeFakeServer>>,
    }

    impl FakeNativeLauncher {
        fn new(call_delay: Duration) -> Arc<Self> {
            Arc::new(Self {
                launches: Arc::new(AtomicUsize::new(0)),
                call_delay,
                current: tokio::sync::Mutex::new(None),
            })
        }

        async fn current_server(&self) -> NativeFakeServer {
            self.current.lock().await.clone().expect("server spawned")
        }
    }

    #[async_trait]
    impl NativeServerLauncher for FakeNativeLauncher {
        async fn spawn(
            &self,
            _distro_name: &str,
            port: u16,
        ) -> Result<SpawnedNativeServer, CoreError> {
            self.launches.fetch_add(1, Ordering::SeqCst);
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
            let server = NativeFakeServer {
                shutdown: Arc::new(tokio::sync::Mutex::new(Some(shutdown_tx))),
                recorded: Arc::new(tokio::sync::Mutex::new(Vec::new())),
                in_flight: Arc::new(AtomicUsize::new(0)),
                max_in_flight: Arc::new(AtomicUsize::new(0)),
                call_delay: self.call_delay,
            };
            let app = axum::Router::new()
                .route("/mcp", axum::routing::post(native_fake_handler))
                .with_state(server.clone());
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .map_err(|e| {
                    CoreError::new(
                        ErrorCode::BrowserStartFailed,
                        format!("fake native server failed to bind: {e}"),
                    )
                })?;
            tokio::spawn(async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        shutdown_rx.await.ok();
                    })
                    .await
                    .unwrap();
            });
            *self.current.lock().await = Some(server);
            Ok(SpawnedNativeServer {
                child: None,
                job: None,
            })
        }
    }

    struct FakeNativeRuntime {
        stops: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl NativeRuntimeGate for FakeNativeRuntime {
        async fn ensure_ready(&self) -> Result<String, CoreError> {
            Ok("TestDistro".to_string())
        }

        async fn stop_server(&self, _distro_name: &str) -> Result<(), CoreError> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct NativeFixture {
        service: Arc<LightpandaNativeService>,
        launcher: Arc<FakeNativeLauncher>,
        stops: Arc<AtomicUsize>,
    }

    fn native_fixture(enabled: bool, call_delay: Duration) -> NativeFixture {
        let stops = Arc::new(AtomicUsize::new(0));
        let launcher = FakeNativeLauncher::new(call_delay);
        let service = LightpandaNativeService::new(
            Arc::new(FakeNativeRuntime {
                stops: stops.clone(),
            }),
            launcher.clone(),
            crate::browser::types::BrowserRuntimeSettings { enabled },
        );
        NativeFixture {
            service,
            launcher,
            stops,
        }
    }

    #[tokio::test]
    async fn native_service_starts_lazily_and_reuses_single_process() {
        let fx = native_fixture(true, Duration::from_millis(0));
        assert!(!fx.service.status().await.running);
        let outcome = fx
            .service
            .call_native("getUrl", Map::new(), None)
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            crate::browser::http::UpstreamToolResponse::Result(_)
        ));
        let status = fx.service.status().await;
        assert!(status.running);
        assert!(status.contract_verified);
        assert_eq!(status.native_tool_count, 31);
        assert_eq!(fx.launcher.launches.load(Ordering::SeqCst), 1);
        fx.service
            .call_native("getUrl", Map::new(), None)
            .await
            .unwrap();
        assert_eq!(fx.launcher.launches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn native_service_disabled_fails_closed_without_launch() {
        let fx = native_fixture(false, Duration::from_millis(0));
        let err = fx
            .service
            .call_native("getUrl", Map::new(), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserAutomationDisabled);
        assert_eq!(fx.launcher.launches.load(Ordering::SeqCst), 0);
        assert!(!fx.service.status().await.running);
    }

    #[tokio::test]
    async fn native_service_pause_stops_running_server_and_blocks_calls() {
        let fx = native_fixture(true, Duration::from_millis(0));
        fx.service
            .call_native("getUrl", Map::new(), None)
            .await
            .unwrap();
        assert!(fx.service.status().await.running);
        fx.service.on_pause_changed(true).await;
        assert!(!fx.service.status().await.running);
        assert_eq!(fx.stops.load(Ordering::SeqCst), 1);
        let err = fx
            .service
            .call_native("getUrl", Map::new(), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserAutomationDisabled);
        // Resuming allows a fresh server on the next independent call.
        fx.service.on_pause_changed(false).await;
        fx.service
            .call_native("getUrl", Map::new(), None)
            .await
            .unwrap();
        assert_eq!(fx.launcher.launches.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn native_service_disable_stops_running_server() {
        let fx = native_fixture(true, Duration::from_millis(0));
        fx.service
            .call_native("getUrl", Map::new(), None)
            .await
            .unwrap();
        fx.service
            .apply_settings(crate::browser::types::BrowserRuntimeSettings { enabled: false })
            .await;
        let status = fx.service.status().await;
        assert!(!status.running);
        assert!(!status.enabled);
        assert_eq!(fx.stops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn native_service_process_loss_is_unhealthy_and_next_call_restarts() {
        let fx = native_fixture(true, Duration::from_millis(0));
        fx.service
            .call_native("getUrl", Map::new(), None)
            .await
            .unwrap();
        let server = fx.launcher.current_server().await;
        server.kill().await;

        let err = fx
            .service
            .call_native("getUrl", Map::new(), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserProcessLost);
        // No automatic replay: only the first successful getUrl call was
        // recorded (the bootstrap session_list from contract verification
        // is not a replayed user call).
        let get_url_calls = server
            .recorded
            .lock()
            .await
            .iter()
            .filter(|(tool, _)| tool == "getUrl")
            .count();
        assert_eq!(get_url_calls, 1);
        assert!(!fx.service.status().await.running);

        // The next independent call starts a fresh server.
        fx.service
            .call_native("getUrl", Map::new(), None)
            .await
            .unwrap();
        assert_eq!(fx.launcher.launches.load(Ordering::SeqCst), 2);
        assert!(fx.service.status().await.running);
    }

    #[tokio::test]
    async fn native_service_serializes_same_session_and_allows_concurrent_sessions() {
        let fx = native_fixture(true, Duration::from_millis(150));
        // Same session: serialized, never overlapping.
        let s1 = fx.service.clone();
        let a =
            tokio::spawn(async move { s1.call_native("getUrl", Map::new(), Some("alpha")).await });
        let s2 = fx.service.clone();
        let b =
            tokio::spawn(async move { s2.call_native("getUrl", Map::new(), Some("alpha")).await });
        let (ra, rb) = tokio::join!(a, b);
        ra.unwrap().unwrap();
        rb.unwrap().unwrap();
        let server = fx.launcher.current_server().await;
        assert_eq!(server.max_in_flight.load(Ordering::SeqCst), 1);

        // Different sessions: may overlap.
        let s3 = fx.service.clone();
        let c =
            tokio::spawn(async move { s3.call_native("getUrl", Map::new(), Some("alpha")).await });
        let s4 = fx.service.clone();
        let d =
            tokio::spawn(async move { s4.call_native("getUrl", Map::new(), Some("beta")).await });
        let (rc, rd) = tokio::join!(c, d);
        rc.unwrap().unwrap();
        rd.unwrap().unwrap();
        assert_eq!(server.max_in_flight.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn native_service_session_control_routes_over_default() {
        let fx = native_fixture(true, Duration::from_millis(0));
        fx.service
            .call_native("session_new", Map::new(), None)
            .await
            .unwrap();
        // Even when a routing session id is supplied, session control stays
        // on the default bootstrap session.
        fx.service
            .call_native("session_close", Map::new(), Some("ignored"))
            .await
            .unwrap();
        let server = fx.launcher.current_server().await;
        let recorded = server.recorded.lock().await.clone();
        // Bootstrap contract verification already ran session_list on default;
        // the explicit session-control calls must also stay on default.
        assert!(recorded
            .iter()
            .all(|(_, session)| session.as_deref() == Some("default")));
        let control: Vec<(String, Option<String>)> = recorded
            .into_iter()
            .filter(|(tool, _)| tool != "session_list")
            .collect();
        assert_eq!(
            control,
            vec![
                ("session_new".to_string(), Some("default".to_string())),
                ("session_close".to_string(), Some("default".to_string())),
            ]
        );
    }
}
