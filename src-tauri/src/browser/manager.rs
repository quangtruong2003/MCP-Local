//! Browser lifecycle, policy, and session state.
//!
//! One logical page = one Lightpanda child stdio MCP process inside the
//! managed WSL2 distro. The manager owns the page table, the opaque page ids,
//! the ref generation counters, and the canonical cookie snapshot. There is no
//! upstream "tabs" primitive: logical tabs are manager-owned state, each
//! backed by its own child process, so one crashing page can never take down
//! its siblings.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use lc_core::{
    error::{CoreError, ErrorCode},
    CoreServices,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};
use tokio::sync::{Mutex, RwLock};

use crate::settings::AppSettings;

use super::{
    artifacts::ArtifactStore,
    driver::{BrowserPageDriver, BrowserPageDriverFactory, PageLaunchSpec},
    mcp::{call_result_text, is_valid_ref, normalize_tree_text, parse_node_selector},
    policy::validate_navigation_url,
    runtime::{BrowserRuntimeManager, RuntimeProvisionState},
    types::{
        BrowserEngine, BrowserPageStatus, BrowserRunMode, BrowserRuntimeSettings,
        BrowserSessionStatus,
    },
    wsl::{WslCommandRunner, WslHostAdapter, WslUser},
};

/// Fixed internal launch identity retained for desktop diagnostics until the
/// Playwright runtime removal. The product backend is pinned to Lightpanda.
const LEGACY_FIXED_ENGINE: BrowserEngine = BrowserEngine::BundledChromium;
const LEGACY_FIXED_RUN_MODE: BrowserRunMode = BrowserRunMode::Background;

/// Shared WSL-owned browser state. Every page child of every session uses the
/// same profile/cookie/cache roots inside the app-owned distro.
pub const BROWSER_PROFILE_DIR: &str = "/var/lib/local-workspaces/browser/profile";
pub const BROWSER_COOKIE_DIR: &str = "/var/lib/local-workspaces/browser/cookies";
pub const BROWSER_CACHE_DIR: &str = "/var/lib/local-workspaces/browser/cache";
pub const STORAGE_SQLITE_PATH: &str = "/var/lib/local-workspaces/browser/profile/storage.sqlite";
pub const CANONICAL_COOKIE_PATH: &str = "/var/lib/local-workspaces/browser/cookies/canonical.json";

/// Per-page exit jar path. Page ids are app-generated opaque values; they are
/// validated here so only a fixed alphabet ever reaches a distro path.
pub fn cookie_exit_jar_path(page_id: &str) -> Result<String, CoreError> {
    let valid = !page_id.is_empty()
        && page_id.len() <= 64
        && page_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "browser page id is not valid",
        ));
    }
    Ok(format!("{BROWSER_COOKIE_DIR}/{page_id}.exit.json"))
}

// ---------------------------------------------------------------------------
// Runtime readiness seam
// ---------------------------------------------------------------------------

/// Gate over runtime readiness (managed WSL distro + pinned Lightpanda
/// binary). Page operations fail closed unless the runtime is ready; status
/// reads never call this and therefore never trigger a download.
#[async_trait]
pub trait RuntimeGate: Send + Sync {
    async fn ensure_ready(&self) -> Result<(), CoreError>;
}

/// Production gate backed by the idempotent runtime provisioner. A runtime
/// that is already `Ready` passes without re-provisioning; anything else runs
/// the full lazy provisioning pipeline once.
pub struct RuntimeManagerGate {
    runtime: Arc<BrowserRuntimeManager>,
}

impl RuntimeManagerGate {
    pub fn new(runtime: Arc<BrowserRuntimeManager>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl RuntimeGate for RuntimeManagerGate {
    async fn ensure_ready(&self) -> Result<(), CoreError> {
        if self.runtime.current_state() == RuntimeProvisionState::Ready {
            return Ok(());
        }
        self.runtime.ensure_runtime().await.map(|_| ())
    }
}

// ---------------------------------------------------------------------------
// Cookie store seam (fixed WSL file operations)
// ---------------------------------------------------------------------------

/// Fixed cookie file operations inside the owned distro. Implementations must
/// never log or return cookie values; paths are application constants or
/// validated page-derived names.
#[async_trait]
pub trait CookieStore: Send + Sync {
    async fn write(&self, path: &str, json: &str) -> Result<(), CoreError>;
    /// Returns `None` when the file does not exist yet.
    async fn read(&self, path: &str) -> Result<Option<String>, CoreError>;
}

/// Production cookie store: fixed `cat`/`tee` execs as the unprivileged
/// runtime user inside the owned distro. The distro name is resolved from the
/// provisioner's persisted identity at call time, never from user input.
pub struct WslCookieStore {
    adapter: Arc<WslHostAdapter<Arc<dyn WslCommandRunner>>>,
    runtime: Arc<BrowserRuntimeManager>,
}

impl WslCookieStore {
    pub fn new(
        adapter: Arc<WslHostAdapter<Arc<dyn WslCommandRunner>>>,
        runtime: Arc<BrowserRuntimeManager>,
    ) -> Self {
        Self { adapter, runtime }
    }

    fn distro_name(&self) -> Result<String, CoreError> {
        self.runtime
            .distro_identity()
            .map(|identity| identity.distro_name)
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::BrowserRuntimeNotReady,
                    "browser runtime is not provisioned yet",
                )
            })
    }
}

#[async_trait]
impl CookieStore for WslCookieStore {
    async fn write(&self, path: &str, json: &str) -> Result<(), CoreError> {
        let distro = self.distro_name()?;
        let output = self
            .adapter
            .exec_fixed(
                &distro,
                WslUser::Runtime,
                &["tee", "--", path],
                Some(json.as_bytes().to_vec()),
            )
            .await?;
        if output.exit_code != 0 {
            return Err(CoreError::new(
                ErrorCode::BrowserArtifactFailed,
                "failed to persist browser cookie state",
            ));
        }
        Ok(())
    }

    async fn read(&self, path: &str) -> Result<Option<String>, CoreError> {
        let distro = self.distro_name()?;
        let output = self
            .adapter
            .exec_fixed(&distro, WslUser::Runtime, &["cat", "--", path], None)
            .await?;
        if output.exit_code != 0 {
            return Ok(None);
        }
        Ok(Some(output.decoded_stdout()))
    }
}

// ---------------------------------------------------------------------------
// Cookie merge (pure)
// ---------------------------------------------------------------------------

fn extract_cookies_array(doc: &Value) -> Option<&Vec<Value>> {
    match doc {
        Value::Array(items) => Some(items),
        Value::Object(map) => map.get("cookies").and_then(Value::as_array),
        _ => None,
    }
}

fn cookie_key(cookie: &Value) -> Option<(String, String, String)> {
    let name = cookie.get("name")?.as_str()?;
    let domain = cookie
        .get("domain")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let path = cookie.get("path").and_then(Value::as_str).unwrap_or("/");
    Some((name.to_string(), domain, path.to_string()))
}

/// Merges a page exit jar into the canonical cookie snapshot. Accepts either
/// a top-level array or an object with a `cookies` array on both sides; the
/// output preserves the canonical document shape (or the jar shape when no
/// canonical snapshot exists yet). Cookies are keyed by (name, domain, path);
/// jar entries win on conflict, canonical order is preserved, and new jar
/// cookies are appended in jar order. Returns `None` when neither side holds
/// a parsable cookie document.
pub(crate) fn merge_cookie_documents(canonical: Option<&str>, jar: Option<&str>) -> Option<String> {
    let canonical_doc = canonical.and_then(|s| serde_json::from_str::<Value>(s).ok());
    let jar_doc = jar.and_then(|s| serde_json::from_str::<Value>(s).ok());
    let canonical_items = canonical_doc
        .as_ref()
        .and_then(extract_cookies_array)
        .cloned()
        .unwrap_or_default();
    let jar_items = jar_doc
        .as_ref()
        .and_then(extract_cookies_array)
        .cloned()
        .unwrap_or_default();
    if canonical_items.is_empty() && jar_items.is_empty() {
        return None;
    }

    let mut merged: Vec<Value> = Vec::with_capacity(canonical_items.len() + jar_items.len());
    let mut index: HashMap<(String, String, String), usize> = HashMap::new();
    for cookie in canonical_items {
        if let Some(key) = cookie_key(&cookie) {
            index.insert(key, merged.len());
        }
        merged.push(cookie);
    }
    for cookie in jar_items {
        match cookie_key(&cookie) {
            Some(key) => match index.get(&key) {
                Some(position) => merged[*position] = cookie,
                None => {
                    index.insert(key, merged.len());
                    merged.push(cookie);
                }
            },
            None => merged.push(cookie),
        }
    }

    let canonical_is_object =
        matches!(&canonical_doc, Some(Value::Object(map)) if map.get("cookies").is_some());
    let jar_is_object =
        matches!(&jar_doc, Some(Value::Object(map)) if map.get("cookies").is_some());
    let output = if canonical_is_object || jar_is_object || canonical_doc.is_some() {
        serde_json::json!({ "cookies": merged })
    } else {
        Value::Array(merged)
    };
    serde_json::to_string(&output).ok()
}

// ---------------------------------------------------------------------------
// Manager state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct PageBinding {
    pub page_id: String,
    pub title: String,
    pub url: String,
    pub active: bool,
}

struct ManagedPage {
    page_id: String,
    driver: Arc<dyn BrowserPageDriver>,
    /// Ref generation counter. Bumped on navigation/reload/restart and major
    /// mutations; every ref registered under an older generation is stale.
    generation: u64,
    /// Opaque ref -> Lightpanda backend node id (current generation only).
    refs: HashMap<String, u64>,
    /// Reverse index keeping one stable ref per backend node per generation.
    backend_ids: HashMap<u64, String>,
    next_ref: u64,
    url: Option<String>,
    title: Option<String>,
    lost: bool,
    cookie_jar_path: String,
}

struct ActiveSession {
    session_id: String,
    artifact_dir: PathBuf,
    pages: Vec<ManagedPage>,
    active_page_id: String,
}

#[derive(Default)]
struct ManagerState {
    active: Option<ActiveSession>,
    tracing: bool,
}

pub struct BrowserManager {
    core: Arc<CoreServices>,
    app_data_dir: PathBuf,
    settings: RwLock<BrowserRuntimeSettings>,
    page_factory: Arc<dyn BrowserPageDriverFactory>,
    runtime_gate: Arc<dyn RuntimeGate>,
    cookie_store: Arc<dyn CookieStore>,
    artifacts: ArtifactStore,
    state: Mutex<ManagerState>,
    next_session: AtomicU64,
    next_page: AtomicU64,
}

impl BrowserManager {
    pub fn new(
        core: Arc<CoreServices>,
        app_data_dir: PathBuf,
        settings: &AppSettings,
        page_factory: Arc<dyn BrowserPageDriverFactory>,
        runtime_gate: Arc<dyn RuntimeGate>,
        cookie_store: Arc<dyn CookieStore>,
    ) -> Result<Self, CoreError> {
        let browser_root = app_data_dir.join("browser");
        let artifacts = ArtifactStore::new(browser_root.join("artifacts"))?;
        Ok(Self {
            core,
            app_data_dir,
            settings: RwLock::new(BrowserRuntimeSettings::from(settings)),
            page_factory,
            runtime_gate,
            cookie_store,
            artifacts,
            state: Mutex::new(ManagerState::default()),
            next_session: AtomicU64::new(1),
            next_page: AtomicU64::new(1),
        })
    }

    /// Production wiring: one Lightpanda page child per logical page inside
    /// the managed WSL distro. Lazy — nothing is provisioned until a page
    /// operation passes the runtime gate.
    #[cfg(windows)]
    pub fn with_lightpanda(
        core: Arc<CoreServices>,
        app_data_dir: PathBuf,
        settings: &AppSettings,
        runtime: Arc<BrowserRuntimeManager>,
    ) -> Result<Self, CoreError> {
        use super::lightpanda::{LightpandaPageDriverFactory, RuntimeResolvedSpawner};
        let cookie_store = Arc::new(WslCookieStore::new(runtime.wsl_adapter(), runtime.clone()));
        let spawner = Arc::new(RuntimeResolvedSpawner::new(runtime.clone()));
        let page_factory = Arc::new(LightpandaPageDriverFactory::new(spawner));
        let runtime_gate = Arc::new(RuntimeManagerGate::new(runtime));
        Self::new(
            core,
            app_data_dir,
            settings,
            page_factory,
            runtime_gate,
            cookie_store,
        )
    }

    #[cfg(not(windows))]
    pub fn with_lightpanda(
        _core: Arc<CoreServices>,
        _app_data_dir: PathBuf,
        _settings: &AppSettings,
        _runtime: Arc<BrowserRuntimeManager>,
    ) -> Result<Self, CoreError> {
        Err(CoreError::new(
            ErrorCode::BrowserWslUnavailable,
            "the lightpanda runtime requires a Windows host",
        ))
    }

    pub fn core(&self) -> &Arc<CoreServices> {
        &self.core
    }

    pub fn artifact_root(&self) -> &Path {
        self.artifacts.root()
    }

    pub async fn runtime_settings(&self) -> BrowserRuntimeSettings {
        *self.settings.read().await
    }

    /// Fixed guard order: Pause -> Browser Automation enabled -> runtime
    /// ready. Session/page checks happen in the caller.
    pub(crate) async fn guard_page_operation(&self) -> Result<BrowserRuntimeSettings, CoreError> {
        if self.core.pause.is_paused() {
            return Err(CoreError::new(
                ErrorCode::AccessPaused,
                "access is paused by the user",
            ));
        }
        let settings = *self.settings.read().await;
        if !settings.enabled {
            return Err(CoreError::new(
                ErrorCode::BrowserAutomationDisabled,
                "Browser Automation is disabled in desktop Settings",
            ));
        }
        self.runtime_gate.ensure_ready().await?;
        Ok(settings)
    }

    /// Guard for structurally unsupported tools: applies the Pause and
    /// enabled gates without the runtime gate, so an unsupported call never
    /// triggers runtime provisioning.
    pub(crate) async fn guard_unsupported(&self) -> Result<(), CoreError> {
        if self.core.pause.is_paused() {
            return Err(CoreError::new(
                ErrorCode::AccessPaused,
                "access is paused by the user",
            ));
        }
        if !self.settings.read().await.enabled {
            return Err(CoreError::new(
                ErrorCode::BrowserAutomationDisabled,
                "Browser Automation is disabled in desktop Settings",
            ));
        }
        Ok(())
    }

    pub(crate) fn engine_slug(engine: BrowserEngine) -> &'static str {
        match engine {
            BrowserEngine::BundledChromium => "bundled-chromium",
            BrowserEngine::Chrome => "chrome",
            BrowserEngine::Edge => "edge",
            BrowserEngine::Firefox => "firefox",
        }
    }

    pub fn profile_dir_for(&self, engine: BrowserEngine) -> PathBuf {
        self.app_data_dir
            .join("browser")
            .join("profiles")
            .join(Self::engine_slug(engine))
    }

    fn new_session_id(&self) -> String {
        let counter = self.next_session.fetch_add(1, Ordering::Relaxed);
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        format!("s-{millis}-{counter}")
    }

    pub(crate) fn new_page_id(&self) -> String {
        format!("p-{}", self.next_page.fetch_add(1, Ordering::Relaxed))
    }

    async fn start_new_session(&self) -> Result<(), CoreError> {
        let session_id = self.new_session_id();
        let artifact_dir = self.artifacts.create_session(&session_id)?;
        let mut state = self.state.lock().await;
        state.active = Some(ActiveSession {
            session_id,
            artifact_dir,
            pages: Vec::new(),
            active_page_id: String::new(),
        });
        state.tracing = false;
        Ok(())
    }

    /// Refreshes the canonical cookie snapshot from the active page before a
    /// new page starts, so newly opened tabs inherit current login cookies.
    /// Best-effort by design: a lost active page must not block new tabs.
    async fn refresh_canonical_from_active(&self) {
        let page_id = {
            let state = self.state.lock().await;
            match state.active.as_ref() {
                Some(session)
                    if !session.active_page_id.is_empty()
                        && session
                            .pages
                            .iter()
                            .any(|p| p.page_id == session.active_page_id && !p.lost) =>
                {
                    Some(session.active_page_id.clone())
                }
                _ => None,
            }
        };
        let Some(page_id) = page_id else { return };
        let mut args = Map::new();
        args.insert("all".into(), Value::Bool(true));
        match self.route_to_page(&page_id, "getCookies", args).await {
            Ok(result) => {
                let json = call_result_text(&result);
                if let Err(error) = self.cookie_store.write(CANONICAL_COOKIE_PATH, &json).await {
                    eprintln!("canonical cookie refresh failed: {error}");
                }
            }
            Err(error) => eprintln!("canonical cookie refresh skipped: {error}"),
        }
    }

    /// Merges one page's exit jar into the canonical snapshot after the child
    /// stopped. Missing jars (page never persisted cookies) are not an error.
    async fn merge_exit_jar(&self, jar_path: &str) {
        let jar = match self.cookie_store.read(jar_path).await {
            Ok(jar) => jar,
            Err(error) => {
                eprintln!("cookie jar read failed: {error}");
                return;
            }
        };
        let Some(jar) = jar else { return };
        let canonical = self
            .cookie_store
            .read(CANONICAL_COOKIE_PATH)
            .await
            .ok()
            .flatten();
        if let Some(merged) = merge_cookie_documents(canonical.as_deref(), Some(&jar)) {
            if let Err(error) = self
                .cookie_store
                .write(CANONICAL_COOKIE_PATH, &merged)
                .await
            {
                eprintln!("canonical cookie merge failed: {error}");
            }
        }
    }

    /// Spawns one Lightpanda child for a new logical page and makes it
    /// active. Requires an existing session.
    async fn spawn_page(&self, url: Option<&str>) -> Result<String, CoreError> {
        self.refresh_canonical_from_active().await;
        let page_id = self.new_page_id();
        let cookie_jar_path = cookie_exit_jar_path(&page_id)?;
        let spec = PageLaunchSpec {
            page_id: page_id.clone(),
            cookie_path: CANONICAL_COOKIE_PATH.to_string(),
            cookie_jar_path: cookie_jar_path.clone(),
            storage_sqlite_path: STORAGE_SQLITE_PATH.to_string(),
            http_cache_dir: BROWSER_CACHE_DIR.to_string(),
        };
        let driver = self.page_factory.create();
        if let Err(error) = driver.start(spec).await {
            let _ = driver.stop().await;
            return Err(error);
        }
        {
            let mut state = self.state.lock().await;
            let session = state.active.as_mut().ok_or_else(|| {
                CoreError::new(ErrorCode::BrowserNotRunning, "browser session ended")
            })?;
            session.pages.push(ManagedPage {
                page_id: page_id.clone(),
                driver,
                generation: 1,
                refs: HashMap::new(),
                backend_ids: HashMap::new(),
                next_ref: 1,
                url: None,
                title: None,
                lost: false,
                cookie_jar_path,
            });
            session.active_page_id = page_id.clone();
        }
        if let Some(url) = url {
            let mut args = Map::new();
            args.insert("url".into(), Value::String(url.to_string()));
            self.route_to_page(&page_id, "goto", args).await?;
            let mut state = self.state.lock().await;
            if let Some(page) = state
                .active
                .as_mut()
                .and_then(|s| s.pages.iter_mut().find(|p| p.page_id == page_id))
            {
                page.url = Some(url.to_string());
                // Navigation replaces major DOM state: every older ref is stale.
                Self::invalidate_page_refs(page);
            }
        }
        Ok(page_id)
    }

    /// Routes one tool call to a specific page child, latching the page as
    /// lost when its connection drops. Never replays; the caller decides.
    async fn route_to_page(
        &self,
        page_id: &str,
        tool: &'static str,
        arguments: Map<String, Value>,
    ) -> Result<CallToolResult, CoreError> {
        let driver = {
            let state = self.state.lock().await;
            let session = state.active.as_ref().ok_or_else(|| {
                CoreError::new(
                    ErrorCode::BrowserNotRunning,
                    "browser is not running; call browser_open first",
                )
            })?;
            let page = session
                .pages
                .iter()
                .find(|p| p.page_id == page_id)
                .ok_or_else(|| {
                    CoreError::new(
                        ErrorCode::BrowserTargetNotFound,
                        "browser page id is not valid for the current session",
                    )
                })?;
            if page.lost {
                return Err(CoreError::new(
                    ErrorCode::BrowserProcessLost,
                    "browser page process was lost",
                ));
            }
            page.driver.clone()
        };
        let result = driver.call(tool, arguments).await;
        if matches!(&result, Err(error) if error.code == ErrorCode::BrowserProcessLost) {
            let mut state = self.state.lock().await;
            if let Some(page) = state
                .active
                .as_mut()
                .and_then(|s| s.pages.iter_mut().find(|p| p.page_id == page_id))
            {
                page.lost = true;
            }
        }
        result
    }

    fn active_page_id_or_err(state: &ManagerState) -> Result<String, CoreError> {
        state
            .active
            .as_ref()
            .filter(|s| !s.active_page_id.is_empty())
            .map(|s| s.active_page_id.clone())
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::BrowserNotRunning,
                    "browser is not running; call browser_open first",
                )
            })
    }

    pub async fn status(&self) -> BrowserSessionStatus {
        let settings = *self.settings.read().await;
        let state = self.state.lock().await;
        let active = state.active.as_ref();
        let active_page = active
            .and_then(|s| s.pages.iter().find(|p| p.page_id == s.active_page_id))
            .map(|p| BrowserPageStatus {
                page_id: p.page_id.clone(),
                url: p.url.clone().unwrap_or_default(),
                title: p.title.clone().unwrap_or_default(),
                viewport_width: None,
                viewport_height: None,
            });
        BrowserSessionStatus {
            enabled: settings.enabled,
            paused: self.core.pause.is_paused(),
            configured_engine: LEGACY_FIXED_ENGINE,
            actual_engine: None,
            run_mode: LEGACY_FIXED_RUN_MODE,
            running: active.is_some(),
            session_id: active.map(|s| s.session_id.clone()),
            active_page,
            emulation: None,
            runtime_version: None,
            browser_version: None,
        }
    }

    pub async fn open(&self, url: Option<&str>) -> Result<BrowserSessionStatus, CoreError> {
        self.guard_page_operation().await?;
        if let Some(url) = url {
            validate_navigation_url(url)?;
        }
        let needs_session = self.state.lock().await.active.is_none();
        if needs_session {
            self.start_new_session().await?;
        }
        let needs_page = self
            .state
            .lock()
            .await
            .active
            .as_ref()
            .is_some_and(|s| s.pages.is_empty());
        if needs_page {
            self.spawn_page(None).await?;
        }
        if let Some(url) = url {
            let page_id = {
                let state = self.state.lock().await;
                Self::active_page_id_or_err(&state)?
            };
            let mut args = Map::new();
            args.insert("url".into(), Value::String(url.to_string()));
            self.route_to_page(&page_id, "goto", args).await?;
            let mut state = self.state.lock().await;
            if let Some(page) = state
                .active
                .as_mut()
                .and_then(|s| s.pages.iter_mut().find(|p| p.page_id == page_id))
            {
                page.url = Some(url.to_string());
                // Navigation replaces major DOM state: every older ref is stale.
                Self::invalidate_page_refs(page);
            }
        }
        Ok(self.status().await)
    }

    async fn stop_all_unchecked(&self) -> Result<(), CoreError> {
        let active = self.state.lock().await.active.take();
        if let Some(active) = active {
            for page in &active.pages {
                let _ = page.driver.stop().await;
                self.merge_exit_jar(&page.cookie_jar_path).await;
            }
            self.artifacts
                .mark_completed(&active.session_id, SystemTime::now())?;
            let _ = self.artifacts.cleanup(SystemTime::now());
        }
        self.state.lock().await.tracing = false;
        Ok(())
    }

    pub async fn close(&self) -> Result<BrowserSessionStatus, CoreError> {
        self.guard_page_operation().await?;
        self.stop_all_unchecked().await?;
        Ok(self.status().await)
    }

    /// Forwards one tool call to the active page child. Transitional routing
    /// surface: Tasks 9-12 replace the remaining Playwright-name callers with
    /// typed native operations and remove this method.
    pub async fn call_upstream(
        &self,
        tool: &'static str,
        arguments: Map<String, Value>,
    ) -> Result<CallToolResult, CoreError> {
        self.guard_page_operation().await?;
        let page_id = {
            let state = self.state.lock().await;
            Self::active_page_id_or_err(&state)?
        };
        self.route_to_page(&page_id, tool, arguments).await
    }

    // -----------------------------------------------------------------------
    // Opaque refs: (ref, page_id, generation) -> backendNodeId. Lightpanda
    // backend node ids never become public identity; every public ref is
    // scoped to one page and one generation and dies with it.
    // -----------------------------------------------------------------------

    fn page_mut<'a>(
        state: &'a mut ManagerState,
        page_id: &str,
    ) -> Result<&'a mut ManagedPage, CoreError> {
        let session = state.active.as_mut().ok_or_else(|| {
            CoreError::new(
                ErrorCode::BrowserNotRunning,
                "browser is not running; call browser_open first",
            )
        })?;
        session
            .pages
            .iter_mut()
            .find(|p| p.page_id == page_id)
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::BrowserTargetNotFound,
                    "browser page id is not valid for the current session",
                )
            })
    }

    /// Registers one backend node id under the page's current generation and
    /// returns its stable opaque ref. The same backend id always maps to the
    /// same ref within one generation.
    fn register_backend_node(page: &mut ManagedPage, backend_node_id: u64) -> String {
        if let Some(existing) = page.backend_ids.get(&backend_node_id) {
            return existing.clone();
        }
        let ref_id = format!("e{}", page.next_ref);
        page.next_ref += 1;
        page.backend_ids.insert(backend_node_id, ref_id.clone());
        page.refs.insert(ref_id.clone(), backend_node_id);
        ref_id
    }

    fn invalidate_page_refs(page: &mut ManagedPage) {
        page.generation += 1;
        page.refs.clear();
        page.backend_ids.clear();
        page.next_ref = 1;
    }

    /// Bumps the page generation, invalidating every ref registered before.
    pub(crate) async fn invalidate_refs(&self, page_id: &str) {
        let mut state = self.state.lock().await;
        if let Ok(page) = Self::page_mut(&mut state, page_id) {
            Self::invalidate_page_refs(page);
        }
    }

    /// Resolves one opaque ref against a page's current generation. Stale and
    /// cross-page refs are indistinguishable on purpose: both fail closed
    /// with BROWSER_STALE_REF before any native action.
    pub(crate) async fn resolve_ref(&self, page_id: &str, ref_id: &str) -> Result<u64, CoreError> {
        if !is_valid_ref(ref_id) {
            return Err(CoreError::new(
                ErrorCode::InvalidArgument,
                "browser ref must look like e1, e2, ...",
            ));
        }
        let state = self.state.lock().await;
        let session = state.active.as_ref().ok_or_else(|| {
            CoreError::new(
                ErrorCode::BrowserNotRunning,
                "browser is not running; call browser_open first",
            )
        })?;
        let page = session
            .pages
            .iter()
            .find(|p| p.page_id == page_id)
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::BrowserTargetNotFound,
                    "browser page id is not valid for the current session",
                )
            })?;
        page.refs.get(ref_id).copied().ok_or_else(|| {
            CoreError::new(
                ErrorCode::BrowserStaleRef,
                "snapshot ref is stale for this page; take a new browser_snapshot",
            )
        })
    }

    /// Recovers the element's current CSS selector via `nodeDetails`. The
    /// backend node id never leaves the manager; a node-gone failure
    /// invalidates the whole generation and surfaces BROWSER_STALE_REF.
    pub(crate) async fn selector_for_ref(
        &self,
        page_id: &str,
        ref_id: &str,
    ) -> Result<String, CoreError> {
        let backend_node_id = self.resolve_ref(page_id, ref_id).await?;
        let mut args = Map::new();
        args.insert("backendNodeId".into(), Value::from(backend_node_id));
        let response = match self.route_to_page(page_id, "nodeDetails", args).await {
            Ok(response) => response,
            Err(error) if error.code == ErrorCode::BrowserProcessLost => return Err(error),
            Err(_) => {
                self.invalidate_refs(page_id).await;
                return Err(CoreError::new(
                    ErrorCode::BrowserStaleRef,
                    "the node behind this ref no longer exists; take a new browser_snapshot",
                ));
            }
        };
        match parse_node_selector(&call_result_text(&response)) {
            Some(selector) => Ok(selector),
            None => {
                self.invalidate_refs(page_id).await;
                Err(CoreError::new(
                    ErrorCode::BrowserStaleRef,
                    "the node behind this ref has no usable selector; take a new browser_snapshot",
                ))
            }
        }
    }

    /// Semantic-tree snapshot of the active page with every backend node id
    /// replaced by a freshly registered opaque ref for the current generation.
    pub async fn snapshot_tree(&self) -> Result<String, CoreError> {
        self.guard_page_operation().await?;
        let page_id = {
            let state = self.state.lock().await;
            Self::active_page_id_or_err(&state)?
        };
        let response = self.route_to_page(&page_id, "tree", Map::new()).await?;
        let text = call_result_text(&response);
        let mut state = self.state.lock().await;
        let page = Self::page_mut(&mut state, &page_id)?;
        Ok(normalize_tree_text(&text, &mut |id| {
            Self::register_backend_node(page, id)
        }))
    }

    /// Resolves ref -> nodeDetails selector -> one native selector-based
    /// action. Refs are invalidated conservatively after the side-effecting
    /// action, even when the action itself fails.
    async fn act_on_ref(
        &self,
        ref_id: &str,
        tool: &'static str,
        mut args: Map<String, Value>,
    ) -> Result<CallToolResult, CoreError> {
        self.guard_page_operation().await?;
        let page_id = {
            let state = self.state.lock().await;
            Self::active_page_id_or_err(&state)?
        };
        let selector = self.selector_for_ref(&page_id, ref_id).await?;
        args.insert("selector".into(), Value::String(selector));
        let result = self.route_to_page(&page_id, tool, args).await;
        self.invalidate_refs(&page_id).await;
        result
    }

    pub async fn click_ref(&self, ref_id: &str) -> Result<CallToolResult, CoreError> {
        self.act_on_ref(ref_id, "click", Map::new()).await
    }

    pub async fn hover_ref(&self, ref_id: &str) -> Result<CallToolResult, CoreError> {
        self.act_on_ref(ref_id, "hover", Map::new()).await
    }

    /// Fills the target and optionally submits with a native Enter press.
    /// Lightpanda `fill` has no slow-typing mode; `slowly` is accepted for
    /// contract compatibility and audited, but has no native effect.
    pub async fn fill_ref(
        &self,
        ref_id: &str,
        text: &str,
        submit: bool,
    ) -> Result<CallToolResult, CoreError> {
        let mut args = Map::new();
        args.insert("value".into(), Value::String(text.to_string()));
        let result = self.act_on_ref(ref_id, "fill", args).await?;
        if submit {
            let page_id = {
                let state = self.state.lock().await;
                Self::active_page_id_or_err(&state)?
            };
            let mut press = Map::new();
            press.insert("key".into(), Value::String("Enter".into()));
            self.route_to_page(&page_id, "press", press).await?;
        }
        Ok(result)
    }

    /// Selects one or more values. Lightpanda `selectOption` accepts one
    /// value per call, so multi-select issues one call per value against the
    /// selector recovered once up front.
    pub async fn select_ref(
        &self,
        ref_id: &str,
        values: &[String],
    ) -> Result<CallToolResult, CoreError> {
        if values.is_empty() {
            return Err(CoreError::new(
                ErrorCode::InvalidArgument,
                "browser_select requires at least one value",
            ));
        }
        self.guard_page_operation().await?;
        let page_id = {
            let state = self.state.lock().await;
            Self::active_page_id_or_err(&state)?
        };
        let selector = self.selector_for_ref(&page_id, ref_id).await?;
        let mut result = CallToolResult::success(Vec::new());
        for value in values {
            let mut args = Map::new();
            args.insert("selector".into(), Value::String(selector.clone()));
            args.insert("value".into(), Value::String(value.clone()));
            result = self.route_to_page(&page_id, "selectOption", args).await?;
        }
        self.invalidate_refs(&page_id).await;
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Native-first content and action tools. Native Lightpanda tool names
    // and argument shapes stay private to the manager; server handlers never
    // construct raw native argument maps.
    // -----------------------------------------------------------------------

    async fn active_page_guarded(&self) -> Result<String, CoreError> {
        self.guard_page_operation().await?;
        let state = self.state.lock().await;
        Self::active_page_id_or_err(&state)
    }

    /// Routes one read-only native content tool that takes no arguments.
    async fn route_content_tool(&self, tool: &'static str) -> Result<CallToolResult, CoreError> {
        let page_id = self.active_page_guarded().await?;
        self.route_to_page(&page_id, tool, Map::new()).await
    }

    pub async fn links_content(&self) -> Result<CallToolResult, CoreError> {
        self.route_content_tool("links").await
    }

    pub async fn interactive_elements_content(&self) -> Result<CallToolResult, CoreError> {
        self.route_content_tool("interactiveElements").await
    }

    pub async fn structured_data_content(&self) -> Result<CallToolResult, CoreError> {
        self.route_content_tool("structuredData").await
    }

    pub async fn detect_forms_content(&self) -> Result<CallToolResult, CoreError> {
        self.route_content_tool("detectForms").await
    }

    /// Renders the page, or one snapshot ref's subtree, as markdown.
    pub async fn markdown_content(
        &self,
        ref_id: Option<&str>,
    ) -> Result<CallToolResult, CoreError> {
        self.scoped_content_tool("markdown", ref_id).await
    }

    /// Dumps raw HTML for the document, or one snapshot ref's outerHTML.
    pub async fn html_content(&self, ref_id: Option<&str>) -> Result<CallToolResult, CoreError> {
        self.scoped_content_tool("html", ref_id).await
    }

    async fn scoped_content_tool(
        &self,
        tool: &'static str,
        ref_id: Option<&str>,
    ) -> Result<CallToolResult, CoreError> {
        let page_id = self.active_page_guarded().await?;
        let mut args = Map::new();
        if let Some(ref_id) = ref_id {
            let selector = self.selector_for_ref(&page_id, ref_id).await?;
            args.insert("selector".into(), Value::String(selector));
        }
        self.route_to_page(&page_id, tool, args).await
    }

    /// Finds interactive elements by role and/or accessible name. Callers
    /// must validate that at least one criterion is present.
    pub async fn find_element(
        &self,
        role: Option<&str>,
        name: Option<&str>,
    ) -> Result<CallToolResult, CoreError> {
        let page_id = self.active_page_guarded().await?;
        let mut args = Map::new();
        if let Some(role) = role.map(str::trim).filter(|r| !r.is_empty()) {
            args.insert("role".into(), Value::String(role.to_string()));
        }
        if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
            args.insert("name".into(), Value::String(name.to_string()));
        }
        self.route_to_page(&page_id, "findElement", args).await
    }

    /// Runs one schema-driven extraction. The schema must already be
    /// validated and encoded by `validate_extract_schema`.
    pub async fn extract_data(&self, encoded_schema: &str) -> Result<CallToolResult, CoreError> {
        let page_id = self.active_page_guarded().await?;
        let mut args = Map::new();
        args.insert("schema".into(), Value::String(encoded_schema.to_string()));
        self.route_to_page(&page_id, "extract", args).await
    }

    /// Scrolls the window, or one snapshot ref's element when a ref is
    /// given. Upstream `scroll` targets elements by backend node id only;
    /// the id never leaves the manager. Scrolling does not mutate the DOM,
    /// so existing refs stay valid.
    pub async fn scroll_page(
        &self,
        ref_id: Option<&str>,
        x: Option<i64>,
        y: Option<i64>,
    ) -> Result<CallToolResult, CoreError> {
        let page_id = self.active_page_guarded().await?;
        let mut args = Map::new();
        if let Some(ref_id) = ref_id {
            let backend_node_id = self.resolve_ref(&page_id, ref_id).await?;
            args.insert("backendNodeId".into(), Value::from(backend_node_id));
        }
        if let Some(x) = x {
            args.insert("x".into(), Value::from(x));
        }
        if let Some(y) = y {
            args.insert("y".into(), Value::from(y));
        }
        self.route_to_page(&page_id, "scroll", args).await
    }

    /// Checks or unchecks a checkbox or radio through its recovered
    /// selector. Side-effecting: refs are invalidated even on failure.
    pub async fn set_checked_ref(
        &self,
        ref_id: &str,
        checked: bool,
    ) -> Result<CallToolResult, CoreError> {
        let mut args = Map::new();
        args.insert("checked".into(), Value::Bool(checked));
        self.act_on_ref(ref_id, "setChecked", args).await
    }

    // -----------------------------------------------------------------------
    // Logical tabs: manager-owned, one Lightpanda child each. There is no
    // upstream tabs primitive.
    // -----------------------------------------------------------------------

    pub(crate) async fn page_summaries(&self) -> Vec<PageBinding> {
        let state = self.state.lock().await;
        state
            .active
            .as_ref()
            .map(|s| {
                s.pages
                    .iter()
                    .map(|page| PageBinding {
                        page_id: page.page_id.clone(),
                        title: page.title.clone().unwrap_or_default(),
                        url: page.url.clone().unwrap_or_default(),
                        active: page.page_id == s.active_page_id,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) async fn tabs_list(&self) -> Result<Vec<PageBinding>, CoreError> {
        self.guard_page_operation().await?;
        Ok(self.page_summaries().await)
    }

    pub(crate) async fn tab_new(&self, url: Option<&str>) -> Result<Vec<PageBinding>, CoreError> {
        self.guard_page_operation().await?;
        if let Some(url) = url {
            validate_navigation_url(url)?;
        }
        {
            let state = self.state.lock().await;
            if state.active.is_none() {
                return Err(CoreError::new(
                    ErrorCode::BrowserNotRunning,
                    "browser is not running; call browser_open first",
                ));
            }
        }
        self.spawn_page(url).await?;
        Ok(self.page_summaries().await)
    }

    pub(crate) async fn tab_select(&self, page_id: &str) -> Result<Vec<PageBinding>, CoreError> {
        self.guard_page_operation().await?;
        let mut state = self.state.lock().await;
        let session = state.active.as_mut().ok_or_else(|| {
            CoreError::new(
                ErrorCode::BrowserNotRunning,
                "browser is not running; call browser_open first",
            )
        })?;
        if !session.pages.iter().any(|p| p.page_id == page_id) {
            return Err(CoreError::new(
                ErrorCode::BrowserTargetNotFound,
                "browser page id is not valid for the current session",
            ));
        }
        session.active_page_id = page_id.to_string();
        drop(state);
        Ok(self.page_summaries().await)
    }

    pub(crate) async fn tab_close(&self, page_id: &str) -> Result<Vec<PageBinding>, CoreError> {
        self.guard_page_operation().await?;
        let removed = {
            let mut state = self.state.lock().await;
            let session = state.active.as_mut().ok_or_else(|| {
                CoreError::new(
                    ErrorCode::BrowserNotRunning,
                    "browser is not running; call browser_open first",
                )
            })?;
            let position = session
                .pages
                .iter()
                .position(|p| p.page_id == page_id)
                .ok_or_else(|| {
                    CoreError::new(
                        ErrorCode::BrowserTargetNotFound,
                        "browser page id is not valid for the current session",
                    )
                })?;
            let page = session.pages.remove(position);
            if session.active_page_id == page_id {
                // Deterministic successor: the page that shifted into the
                // closed slot, or the new last page when the tail was closed.
                session.active_page_id = session
                    .pages
                    .get(position)
                    .or_else(|| session.pages.last())
                    .map(|p| p.page_id.clone())
                    .unwrap_or_default();
            }
            page
        };
        let _ = removed.driver.stop().await;
        self.merge_exit_jar(&removed.cookie_jar_path).await;
        let empty = self
            .state
            .lock()
            .await
            .active
            .as_ref()
            .is_some_and(|s| s.pages.is_empty());
        if empty {
            self.stop_all_unchecked().await?;
        }
        Ok(self.page_summaries().await)
    }

    // -----------------------------------------------------------------------
    // Settings / pause integration
    // -----------------------------------------------------------------------

    pub async fn apply_settings(
        &self,
        settings: BrowserRuntimeSettings,
    ) -> Result<BrowserSessionStatus, CoreError> {
        let previous = {
            let mut current = self.settings.write().await;
            let previous = *current;
            *current = settings;
            previous
        };
        let running = self.state.lock().await.active.is_some();
        if running && !settings.enabled && previous.enabled {
            // Settings were already committed in-memory above. Even if stop
            // fails, OFF governs every subsequent guard.
            self.stop_all_unchecked().await?;
        }
        Ok(self.status().await)
    }

    pub async fn on_pause_changed(&self, paused: bool) -> Result<(), CoreError> {
        if paused {
            self.stop_all_unchecked().await?;
        }
        Ok(())
    }

    pub async fn reset_profile(&self, engine: BrowserEngine) -> Result<(), CoreError> {
        // Desktop-only caller handles confirmation. Legacy Windows profile
        // reset retained until the desktop controls move to the WSL-owned
        // browser data reset (Task 13).
        self.stop_all_unchecked().await?;
        let profile = self.profile_dir_for(engine);
        let profiles_root = self.app_data_dir.join("browser").join("profiles");
        std::fs::create_dir_all(&profiles_root).map_err(CoreError::io)?;
        let profiles_root = dunce::canonicalize(&profiles_root).map_err(CoreError::io)?;
        if profile.exists() {
            let canonical = dunce::canonicalize(&profile).map_err(CoreError::io)?;
            if canonical.parent() != Some(profiles_root.as_path()) {
                return Err(CoreError::new(
                    ErrorCode::BrowserArtifactFailed,
                    "browser profile path escaped the managed profile root",
                ));
            }
            std::fs::remove_dir_all(&canonical).map_err(CoreError::io)?;
        }
        std::fs::create_dir_all(&profile).map_err(CoreError::io)
    }

    pub(crate) async fn active_artifact_dir(&self) -> Result<PathBuf, CoreError> {
        self.guard_page_operation().await?;
        self.state
            .lock()
            .await
            .active
            .as_ref()
            .map(|s| s.artifact_dir.clone())
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::BrowserNotRunning,
                    "browser is not running; call browser_open first",
                )
            })
    }

    pub(crate) async fn set_tracing(&self, tracing: bool) {
        self.state.lock().await.tracing = tracing;
    }

    pub(crate) async fn is_tracing(&self) -> bool {
        self.state.lock().await.tracing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use rmcp::model::{CallToolResult, ContentBlock};

    use super::super::driver::PageDriverStatus;

    /// (driver index, tool name, arguments) for every call across all pages.
    type RecordedCall = (usize, String, Map<String, Value>);

    #[derive(Default)]
    struct Recorder {
        created: AtomicUsize,
        starts: AtomicUsize,
        stops: AtomicUsize,
        calls: tokio::sync::Mutex<Vec<RecordedCall>>,
        launch_specs: tokio::sync::Mutex<Vec<PageLaunchSpec>>,
        /// The Nth created driver fails its first call with process lost.
        crash_driver: tokio::sync::Mutex<Option<usize>>,
    }

    struct FakePageDriver {
        index: usize,
        recorder: Arc<Recorder>,
    }

    #[async_trait]
    impl BrowserPageDriver for FakePageDriver {
        async fn start(&self, spec: PageLaunchSpec) -> Result<PageDriverStatus, CoreError> {
            self.recorder.starts.fetch_add(1, Ordering::SeqCst);
            self.recorder.launch_specs.lock().await.push(spec.clone());
            Ok(PageDriverStatus {
                running: true,
                page_id: spec.page_id,
                child_pid: None,
            })
        }

        async fn call(
            &self,
            tool: &'static str,
            arguments: Map<String, Value>,
        ) -> Result<CallToolResult, CoreError> {
            let details_id = arguments
                .get("backendNodeId")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            // Record before the crash check: a crashed call still reached the
            // child and must be observable to the tests.
            self.recorder
                .calls
                .lock()
                .await
                .push((self.index, tool.to_string(), arguments));
            let crash = {
                let mut guard = self.recorder.crash_driver.lock().await;
                match *guard {
                    Some(n) if n == self.index => {
                        *guard = None;
                        true
                    }
                    _ => false,
                }
            };
            if crash {
                return Err(CoreError::new(
                    ErrorCode::BrowserProcessLost,
                    "page process was lost",
                ));
            }
            if tool == "getCookies" {
                return Ok(CallToolResult::success(vec![ContentBlock::text(
                    "{\"cookies\":[{\"name\":\"sid\",\"domain\":\"a.test\",\"path\":\"/\",\"value\":\"v\"}]}",
                )]));
            }
            if tool == "tree" {
                return Ok(CallToolResult::success(vec![ContentBlock::text(
                    "1 WebArea 'x'\n 2 [i] button 'Go'\n",
                )]));
            }
            if tool == "nodeDetails" {
                return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                    "{{\"backendNodeId\":{details_id},\"selector\":\"#nd-{details_id}\"}}"
                ))]));
            }
            Ok(CallToolResult::success(vec![ContentBlock::text("ok")]))
        }

        async fn stop(&self) -> Result<(), CoreError> {
            self.recorder.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn is_alive(&self) -> bool {
            true
        }
    }

    struct FakeFactory {
        recorder: Arc<Recorder>,
    }

    impl BrowserPageDriverFactory for FakeFactory {
        fn create(&self) -> Arc<dyn BrowserPageDriver> {
            let index = self.recorder.created.fetch_add(1, Ordering::SeqCst);
            Arc::new(FakePageDriver {
                index,
                recorder: self.recorder.clone(),
            })
        }
    }

    struct FakeGate {
        result: tokio::sync::Mutex<Option<ErrorCode>>,
    }

    impl FakeGate {
        fn ok() -> Arc<Self> {
            Arc::new(Self {
                result: tokio::sync::Mutex::new(None),
            })
        }

        fn failing(code: ErrorCode) -> Arc<Self> {
            Arc::new(Self {
                result: tokio::sync::Mutex::new(Some(code)),
            })
        }
    }

    #[async_trait]
    impl RuntimeGate for FakeGate {
        async fn ensure_ready(&self) -> Result<(), CoreError> {
            match *self.result.lock().await {
                Some(code) => Err(CoreError::new(code, "runtime not ready")),
                None => Ok(()),
            }
        }
    }

    #[derive(Default)]
    struct FakeCookieStore {
        files: tokio::sync::Mutex<HashMap<String, String>>,
        writes: tokio::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl CookieStore for FakeCookieStore {
        async fn write(&self, path: &str, json: &str) -> Result<(), CoreError> {
            self.files
                .lock()
                .await
                .insert(path.to_string(), json.to_string());
            self.writes.lock().await.push(path.to_string());
            Ok(())
        }

        async fn read(&self, path: &str) -> Result<Option<String>, CoreError> {
            Ok(self.files.lock().await.get(path).cloned())
        }
    }

    fn harness(
        gate: Arc<FakeGate>,
    ) -> (
        BrowserManager,
        Arc<Recorder>,
        Arc<FakeCookieStore>,
        Arc<CoreServices>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(CoreServices::open(&dir.path().join("manager.db")).unwrap());
        let recorder = Arc::new(Recorder::default());
        let factory = Arc::new(FakeFactory {
            recorder: recorder.clone(),
        });
        let cookies = Arc::new(FakeCookieStore::default());
        let manager = BrowserManager::new(
            core.clone(),
            dir.path().into(),
            &AppSettings::default(),
            factory,
            gate,
            cookies.clone(),
        )
        .unwrap();
        (manager, recorder, cookies, core, dir)
    }

    async fn recorded_calls(recorder: &Recorder) -> Vec<(usize, String)> {
        recorder
            .calls
            .lock()
            .await
            .iter()
            .map(|(index, tool, _)| (*index, tool.clone()))
            .collect()
    }

    #[tokio::test]
    async fn guard_order_is_pause_then_enabled_then_runtime() {
        let (manager, _, _, _core, _dir) =
            harness(FakeGate::failing(ErrorCode::BrowserRuntimeNotReady));
        // Runtime gate failure surfaces when pause/enabled pass.
        assert_eq!(
            manager.open(None).await.unwrap_err().code,
            ErrorCode::BrowserRuntimeNotReady
        );

        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(CoreServices::open(&dir.path().join("manager.db")).unwrap());
        let settings = AppSettings {
            browser_automation_enabled: false,
            ..Default::default()
        };
        let recorder = Arc::new(Recorder::default());
        let disabled = BrowserManager::new(
            core.clone(),
            dir.path().into(),
            &settings,
            Arc::new(FakeFactory {
                recorder: recorder.clone(),
            }),
            FakeGate::failing(ErrorCode::BrowserRuntimeNotReady),
            Arc::new(FakeCookieStore::default()),
        )
        .unwrap();
        // Disabled wins over the runtime gate.
        assert_eq!(
            disabled.open(None).await.unwrap_err().code,
            ErrorCode::BrowserAutomationDisabled
        );
        // Pause wins over disabled.
        core.pause.set(true);
        assert_eq!(
            disabled.open(None).await.unwrap_err().code,
            ErrorCode::AccessPaused
        );
    }

    #[tokio::test]
    async fn open_creates_exactly_one_page_child_and_navigates_with_goto() {
        let (manager, recorder, _, _core, _dir) = harness(FakeGate::ok());
        manager.open(Some("https://a.test/")).await.unwrap();
        assert_eq!(recorder.created.load(Ordering::SeqCst), 1);
        assert_eq!(recorder.starts.load(Ordering::SeqCst), 1);
        let calls = recorded_calls(&recorder).await;
        assert_eq!(calls, vec![(0, "goto".to_string())]);
        let status = manager.status().await;
        assert!(status.running);
        assert_eq!(
            status.active_page.as_ref().map(|p| p.url.as_str()),
            Some("https://a.test/")
        );
    }

    #[tokio::test]
    async fn tab_new_creates_second_child_after_refreshing_canonical_cookies() {
        let (manager, recorder, cookies, _core, _dir) = harness(FakeGate::ok());
        manager.open(None).await.unwrap();
        manager.tab_new(Some("https://b.test/")).await.unwrap();

        assert_eq!(recorder.created.load(Ordering::SeqCst), 2);
        let calls = recorded_calls(&recorder).await;
        // The active page is asked for its cookies before the new child
        // starts; the new page is navigated with goto.
        assert!(calls.contains(&(0, "getCookies".to_string())));
        assert!(calls.contains(&(1, "goto".to_string())));
        let get_cookies_index = calls.iter().position(|c| c.1 == "getCookies").unwrap();
        let goto_index = calls.iter().position(|c| c.1 == "goto").unwrap();
        assert!(get_cookies_index < goto_index);
        // Canonical snapshot was written from the active page's cookies.
        let files = cookies.files.lock().await;
        assert!(files.get(CANONICAL_COOKIE_PATH).unwrap().contains("sid"));
        // The new page launched with the canonical cookie path and its own
        // exit jar.
        let specs = recorder.launch_specs.lock().await;
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[1].cookie_path, CANONICAL_COOKIE_PATH);
        assert!(specs[1].cookie_jar_path.ends_with(".exit.json"));
        assert_ne!(specs[0].cookie_jar_path, specs[1].cookie_jar_path);
        assert_eq!(specs[1].storage_sqlite_path, STORAGE_SQLITE_PATH);
    }

    #[tokio::test]
    async fn tab_select_changes_active_without_spawning_or_restarting() {
        let (manager, recorder, _, _core, _dir) = harness(FakeGate::ok());
        manager.open(None).await.unwrap();
        let pages = manager.tab_new(None).await.unwrap();
        let first = pages[0].page_id.clone();

        let selected = manager.tab_select(&first).await.unwrap();
        assert!(selected.iter().find(|p| p.page_id == first).unwrap().active);
        assert_eq!(recorder.created.load(Ordering::SeqCst), 2);
        assert_eq!(recorder.starts.load(Ordering::SeqCst), 2);
        assert_eq!(recorder.stops.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn tab_close_stops_only_that_child_and_merges_its_jar() {
        let (manager, recorder, cookies, _core, _dir) = harness(FakeGate::ok());
        manager.open(None).await.unwrap();
        let pages = manager.tab_new(None).await.unwrap();
        let first = pages[0].page_id.clone();
        let second = pages[1].page_id.clone();

        // Seed the closed page's exit jar.
        let jar_path = cookie_exit_jar_path(&first).unwrap();
        cookies.files.lock().await.insert(
            jar_path.clone(),
            "[{\"name\":\"jar-cookie\",\"domain\":\"b.test\",\"path\":\"/\"}]".into(),
        );

        let remaining = manager.tab_close(&first).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].page_id, second);
        assert!(remaining[0].active, "successor page becomes active");
        assert_eq!(recorder.stops.load(Ordering::SeqCst), 1);
        // Jar contents were merged into the canonical snapshot.
        let files = cookies.files.lock().await;
        let canonical = files.get(CANONICAL_COOKIE_PATH).unwrap();
        assert!(canonical.contains("jar-cookie"));
    }

    #[tokio::test]
    async fn closing_the_last_page_ends_the_session() {
        let (manager, recorder, _, _core, _dir) = harness(FakeGate::ok());
        manager.open(None).await.unwrap();
        let pages = manager.tabs_list().await.unwrap();
        manager.tab_close(&pages[0].page_id).await.unwrap();
        assert!(!manager.status().await.running);
        assert_eq!(recorder.stops.load(Ordering::SeqCst), 1);
        assert!(manager.tabs_list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn one_crashed_page_is_lost_but_siblings_keep_working() {
        let (manager, recorder, _, _core, _dir) = harness(FakeGate::ok());
        manager.open(None).await.unwrap();
        let pages = manager.tab_new(None).await.unwrap();
        let first = pages[0].page_id.clone();
        let second = pages[1].page_id.clone();

        // Crash page 0 while it is active.
        manager.tab_select(&first).await.unwrap();
        *recorder.crash_driver.lock().await = Some(0);
        let err = manager.call_upstream("tree", Map::new()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserProcessLost);
        // The lost page fails fast on subsequent calls.
        assert_eq!(
            manager
                .call_upstream("tree", Map::new())
                .await
                .unwrap_err()
                .code,
            ErrorCode::BrowserProcessLost
        );

        // The sibling page still works.
        manager.tab_select(&second).await.unwrap();
        assert!(manager.call_upstream("tree", Map::new()).await.is_ok());
        let pages = manager.tabs_list().await.unwrap();
        assert_eq!(pages.len(), 2);
    }

    #[tokio::test]
    async fn pause_disable_and_close_stop_every_child() {
        // Pause path.
        let (manager, recorder, _, core, _dir) = harness(FakeGate::ok());
        manager.open(None).await.unwrap();
        manager.tab_new(None).await.unwrap();
        core.pause.set(true);
        manager.on_pause_changed(true).await.unwrap();
        assert_eq!(recorder.stops.load(Ordering::SeqCst), 2);
        assert!(!manager.status().await.running);

        // Disable path.
        let (manager, recorder, _, _core, _dir) = harness(FakeGate::ok());
        manager.open(None).await.unwrap();
        manager.tab_new(None).await.unwrap();
        manager
            .apply_settings(BrowserRuntimeSettings { enabled: false })
            .await
            .unwrap();
        assert_eq!(recorder.stops.load(Ordering::SeqCst), 2);

        // Close path.
        let (manager, recorder, _, _core, _dir) = harness(FakeGate::ok());
        manager.open(None).await.unwrap();
        manager.tab_new(None).await.unwrap();
        manager.close().await.unwrap();
        assert_eq!(recorder.stops.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn tabs_never_call_an_upstream_tabs_primitive() {
        let (manager, recorder, _, _core, _dir) = harness(FakeGate::ok());
        manager.open(None).await.unwrap();
        let pages = manager.tab_new(None).await.unwrap();
        let first = pages[0].page_id.clone();
        manager.tab_select(&first).await.unwrap();
        manager.tabs_list().await.unwrap();
        manager.tab_close(&pages[1].page_id).await.unwrap();

        for (_, tool) in recorded_calls(&recorder).await {
            assert!(
                tool != "browser_tabs" && tool != "tabs",
                "logical tabs must not call an upstream tabs primitive, got {tool}"
            );
        }
    }

    #[tokio::test]
    async fn sibling_pages_do_not_receive_live_cookie_updates() {
        // Documented v0.2.0 limitation: refreshing the canonical snapshot for
        // a new tab must not restart or re-launch already-running siblings;
        // only newly launched pages see the refreshed cookies.
        let (manager, recorder, _, _core, _dir) = harness(FakeGate::ok());
        manager.open(None).await.unwrap();
        manager.tab_new(None).await.unwrap();
        manager.tab_new(None).await.unwrap();

        assert_eq!(recorder.created.load(Ordering::SeqCst), 3);
        assert_eq!(recorder.starts.load(Ordering::SeqCst), 3);
        // No page driver was started twice.
        let specs = recorder.launch_specs.lock().await;
        let mut ids: Vec<&str> = specs.iter().map(|s| s.page_id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3);
    }

    #[tokio::test]
    async fn cookie_merge_is_keyed_deterministic_and_jar_wins() {
        let canonical = r#"{"cookies":[
            {"name":"sid","domain":"A.test","path":"/","value":"old"},
            {"name":"keep","domain":"a.test","path":"/","value":"k"}
        ]}"#;
        let jar = r#"[
            {"name":"sid","domain":"a.test","path":"/","value":"new"},
            {"name":"fresh","domain":"b.test","path":"/x","value":"f"}
        ]"#;
        let merged = merge_cookie_documents(Some(canonical), Some(jar)).unwrap();
        let doc: Value = serde_json::from_str(&merged).unwrap();
        let items = doc.get("cookies").and_then(Value::as_array).unwrap();
        assert_eq!(items.len(), 3);
        // Canonical order preserved; jar wins the (name,domain,path) conflict
        // case-insensitively on domain; new jar cookies append.
        assert_eq!(items[0]["name"], "sid");
        assert_eq!(items[0]["value"], "new");
        assert_eq!(items[1]["name"], "keep");
        assert_eq!(items[2]["name"], "fresh");

        // Jar-only merge yields an array document.
        let only_jar = merge_cookie_documents(None, Some(jar)).unwrap();
        assert!(only_jar.starts_with('['));
        // Nothing parsable on either side merges to nothing.
        assert!(merge_cookie_documents(None, None).is_none());
        assert!(merge_cookie_documents(Some("not json"), None).is_none());
    }

    #[tokio::test]
    async fn status_is_readable_without_runtime_provisioning() {
        let (manager, _, _, _core, _dir) =
            harness(FakeGate::failing(ErrorCode::BrowserRuntimeNotReady));
        let status = manager.status().await;
        assert!(status.enabled);
        assert!(!status.running);
        assert!(status.session_id.is_none());
    }

    #[tokio::test]
    async fn snapshot_registers_refs_and_navigation_invalidates_them() {
        let (manager, _recorder, _store, _core, _dir) = harness(FakeGate::ok());
        manager.open(None).await.unwrap();
        let tree = manager.snapshot_tree().await.unwrap();
        assert!(tree.contains("e1 WebArea"));
        assert!(tree.contains("e2 [i] button"));
        assert_eq!(manager.resolve_ref("p-1", "e2").await.unwrap(), 2);

        // Navigation bumps the generation: the same ref text is now stale.
        manager
            .open(Some("https://example.test/next"))
            .await
            .unwrap();
        assert_eq!(
            manager.resolve_ref("p-1", "e2").await.unwrap_err().code,
            ErrorCode::BrowserStaleRef
        );

        // A fresh snapshot re-registers from e1 in the new generation.
        manager.snapshot_tree().await.unwrap();
        assert_eq!(manager.resolve_ref("p-1", "e1").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn process_loss_during_selector_recovery_surfaces_process_lost() {
        let (manager, recorder, _store, _core, _dir) = harness(FakeGate::ok());
        manager.open(None).await.unwrap();
        manager.snapshot_tree().await.unwrap();
        // Crash the page while nodeDetails is in flight: the caller must see
        // BROWSER_PROCESS_LOST, never a stale-ref rewrite or a replay.
        *recorder.crash_driver.lock().await = Some(0);
        let err = manager.click_ref("e2").await.unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserProcessLost);
        let calls = recorded_calls(&recorder).await;
        assert!(calls.iter().any(|(_, tool)| tool == "nodeDetails"));
        assert!(
            calls.iter().all(|(_, tool)| tool != "click"),
            "no native action may run after process loss"
        );
    }
}
