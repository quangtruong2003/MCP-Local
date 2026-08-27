//! Browser lifecycle, policy, and session state.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

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
    driver::{BrowserDriver, BrowserDriverStatus},
    playwright::PlaywrightMcpDriver,
    policy::validate_navigation_url,
    types::{
        BrowserEmulation, BrowserEngine, BrowserLaunchSpec, BrowserPageStatus, BrowserRunMode,
        BrowserRuntimeSettings, BrowserSessionStatus, DEFAULT_ACTION_TIMEOUT_MS,
        DEFAULT_NAVIGATION_TIMEOUT_MS,
    },
};

#[derive(Debug, Clone)]
pub(crate) struct PageBinding {
    pub page_id: String,
    pub index: usize,
    pub title: String,
    pub url: String,
    pub active: bool,
}

/// Fixed internal launch identity for the bundled Playwright runtime. User
/// engine/run-mode choice was removed with the v6 settings schema; the backend
/// is pinned to the bundled Chromium in background mode until the Lightpanda
/// page driver replaces this runtime entirely.
const LEGACY_FIXED_ENGINE: BrowserEngine = BrowserEngine::BundledChromium;
const LEGACY_FIXED_RUN_MODE: BrowserRunMode = BrowserRunMode::Background;

#[derive(Debug, Clone)]
struct ActiveSession {
    session_id: String,
    launch: BrowserLaunchSpec,
    driver: BrowserDriverStatus,
    pages: HashMap<String, PageBinding>,
    active_page_id: Option<String>,
}

#[derive(Debug, Default)]
struct ManagerState {
    active: Option<ActiveSession>,
    pending_emulation: Option<BrowserEmulation>,
    tracing: bool,
}

pub struct BrowserManager {
    core: Arc<CoreServices>,
    app_data_dir: PathBuf,
    settings: RwLock<BrowserRuntimeSettings>,
    driver: Arc<dyn BrowserDriver>,
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
        driver: Arc<dyn BrowserDriver>,
    ) -> Result<Self, CoreError> {
        let browser_root = app_data_dir.join("browser");
        let artifacts = ArtifactStore::new(browser_root.join("artifacts"))?;
        std::fs::create_dir_all(browser_root.join("profiles")).map_err(CoreError::io)?;
        Ok(Self {
            core,
            app_data_dir,
            settings: RwLock::new(BrowserRuntimeSettings::from(settings)),
            driver,
            artifacts,
            state: Mutex::new(ManagerState::default()),
            next_session: AtomicU64::new(1),
            next_page: AtomicU64::new(1),
        })
    }

    pub fn with_playwright(
        core: Arc<CoreServices>,
        app_data_dir: PathBuf,
        runtime_root: PathBuf,
        settings: &AppSettings,
    ) -> Result<Self, CoreError> {
        Self::new(
            core,
            app_data_dir,
            settings,
            Arc::new(PlaywrightMcpDriver::new(runtime_root)),
        )
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
        Ok(settings)
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

    async fn start_new_session(
        &self,
        _settings: BrowserRuntimeSettings,
        emulation: Option<BrowserEmulation>,
    ) -> Result<BrowserSessionStatus, CoreError> {
        let session_id = self.new_session_id();
        let artifact_dir = self.artifacts.create_session(&session_id)?;
        let profile_dir = self.profile_dir_for(LEGACY_FIXED_ENGINE);
        std::fs::create_dir_all(&profile_dir).map_err(CoreError::io)?;
        let launch = BrowserLaunchSpec {
            session_id: session_id.clone(),
            engine: LEGACY_FIXED_ENGINE,
            run_mode: LEGACY_FIXED_RUN_MODE,
            profile_dir,
            artifact_dir,
            action_timeout_ms: DEFAULT_ACTION_TIMEOUT_MS,
            navigation_timeout_ms: DEFAULT_NAVIGATION_TIMEOUT_MS,
            emulation: emulation.clone(),
        };
        let driver_status = match self.driver.start(launch.clone()).await {
            Ok(status) => status,
            Err(error) => {
                let _ = self
                    .artifacts
                    .mark_completed(&session_id, SystemTime::now());
                return Err(error);
            }
        };
        let mut state = self.state.lock().await;
        state.active = Some(ActiveSession {
            session_id,
            launch,
            driver: driver_status,
            pages: HashMap::new(),
            active_page_id: None,
        });
        state.tracing = false;
        drop(state);
        Ok(self.status().await)
    }

    pub async fn status(&self) -> BrowserSessionStatus {
        let settings = *self.settings.read().await;
        let state = self.state.lock().await;
        let active = state.active.as_ref();
        let active_page = active
            .and_then(|s| s.active_page_id.as_ref().and_then(|id| s.pages.get(id)))
            .map(|p| BrowserPageStatus {
                page_id: p.page_id.clone(),
                url: p.url.clone(),
                title: p.title.clone(),
                viewport_width: active.and_then(|s| s.launch.emulation.as_ref().map(|e| e.width)),
                viewport_height: active.and_then(|s| s.launch.emulation.as_ref().map(|e| e.height)),
            });
        BrowserSessionStatus {
            enabled: settings.enabled,
            paused: self.core.pause.is_paused(),
            configured_engine: LEGACY_FIXED_ENGINE,
            actual_engine: active.map(|s| s.driver.actual_engine),
            run_mode: LEGACY_FIXED_RUN_MODE,
            running: active.is_some(),
            session_id: active.map(|s| s.session_id.clone()),
            active_page,
            emulation: active
                .and_then(|s| s.launch.emulation.clone())
                .or_else(|| state.pending_emulation.clone()),
            runtime_version: active.and_then(|s| s.driver.sidecar_version.clone()),
            browser_version: active.and_then(|s| s.driver.browser_version.clone()),
        }
    }

    pub async fn open(&self, url: Option<&str>) -> Result<BrowserSessionStatus, CoreError> {
        let settings = self.guard_page_operation().await?;
        if let Some(url) = url {
            validate_navigation_url(url)?;
        }
        let running = self.state.lock().await.active.is_some();
        if !running {
            let emulation = self.state.lock().await.pending_emulation.clone();
            self.start_new_session(settings, emulation).await?;
        }
        if let Some(url) = url {
            let mut args = Map::new();
            args.insert("url".into(), Value::String(url.to_string()));
            self.call_upstream("browser_navigate", args).await?;
        }
        Ok(self.status().await)
    }

    async fn stop_active_unchecked(&self) -> Result<(), CoreError> {
        let active = self.state.lock().await.active.take();
        if let Some(active) = active {
            self.driver.stop().await?;
            self.artifacts
                .mark_completed(&active.session_id, SystemTime::now())?;
            let _ = self.artifacts.cleanup(SystemTime::now());
        }
        self.state.lock().await.tracing = false;
        Ok(())
    }

    pub async fn close(&self) -> Result<BrowserSessionStatus, CoreError> {
        self.guard_page_operation().await?;
        self.stop_active_unchecked().await?;
        Ok(self.status().await)
    }

    pub async fn call_upstream(
        &self,
        tool: &'static str,
        arguments: Map<String, Value>,
    ) -> Result<CallToolResult, CoreError> {
        self.guard_page_operation().await?;
        if self.state.lock().await.active.is_none() {
            return Err(CoreError::new(
                ErrorCode::BrowserNotRunning,
                "browser is not running; call browser_open first",
            ));
        }
        self.driver.call(tool, arguments).await
    }

    pub async fn chromium_performance_metrics(&self) -> Result<Value, CoreError> {
        self.guard_page_operation().await?;
        if self.state.lock().await.active.is_none() {
            return Err(CoreError::new(
                ErrorCode::BrowserNotRunning,
                "browser is not running; call browser_open first",
            ));
        }
        self.driver.chromium_performance_metrics().await
    }

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
            self.stop_active_unchecked().await?;
        }
        Ok(self.status().await)
    }

    pub async fn on_pause_changed(&self, paused: bool) -> Result<(), CoreError> {
        if paused {
            self.stop_active_unchecked().await?;
        }
        Ok(())
    }

    pub async fn restart_for_emulation(
        &self,
        emulation: BrowserEmulation,
    ) -> Result<BrowserSessionStatus, CoreError> {
        let settings = self.guard_page_operation().await?;
        let was_running = self.state.lock().await.active.is_some();
        self.state.lock().await.pending_emulation = Some(emulation.clone());
        if was_running {
            self.stop_active_unchecked().await?;
            self.start_new_session(settings, Some(emulation)).await
        } else {
            Ok(self.status().await)
        }
    }

    pub async fn reset_profile(&self, engine: BrowserEngine) -> Result<(), CoreError> {
        // Desktop-only caller handles confirmation. Stop first so profile files
        // are not locked by Chromium/Firefox.
        self.stop_active_unchecked().await?;
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

    pub(crate) async fn set_pages(&self, pages: Vec<PageBinding>) {
        let mut state = self.state.lock().await;
        if let Some(active) = state.active.as_mut() {
            active.pages = pages.into_iter().map(|p| (p.page_id.clone(), p)).collect();
            active.active_page_id = active
                .pages
                .values()
                .find(|p| p.active)
                .map(|p| p.page_id.clone());
        }
    }

    pub(crate) async fn page_bindings(&self) -> Vec<PageBinding> {
        let state = self.state.lock().await;
        let mut pages: Vec<PageBinding> = state
            .active
            .as_ref()
            .map(|s| s.pages.values().cloned().collect())
            .unwrap_or_default();
        pages.sort_by_key(|page| page.index);
        pages
    }

    pub(crate) async fn forget_page(&self, page_id: &str) {
        let mut state = self.state.lock().await;
        if let Some(active) = state.active.as_mut() {
            active.pages.remove(page_id);
            if active.active_page_id.as_deref() == Some(page_id) {
                active.active_page_id = None;
            }
        }
    }

    pub(crate) async fn reconcile_pages(
        &self,
        tabs: Vec<(usize, bool, String, String)>,
    ) -> Vec<PageBinding> {
        let previous = self.page_bindings().await;
        let mut previous_keys: HashMap<(String, String), usize> = HashMap::new();
        for page in &previous {
            *previous_keys
                .entry((page.url.clone(), page.title.clone()))
                .or_default() += 1;
        }
        let mut incoming_keys: HashMap<(String, String), usize> = HashMap::new();
        for (_, _, title, url) in &tabs {
            *incoming_keys
                .entry((url.clone(), title.clone()))
                .or_default() += 1;
        }
        let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut slots: Vec<Option<PageBinding>> = vec![None; tabs.len()];
        // Pass 1: match on (url, title) only when that key is unique on both
        // sides. Duplicate tabs are ambiguous, so they must never drive ID
        // swapping; unique keys keep their ids across index shifts (e.g. an
        // earlier tab was closed).
        for (slot, (index, active, title, url)) in tabs.iter().enumerate() {
            let key = (url.clone(), title.clone());
            let unique = previous_keys.get(&key) == Some(&1) && incoming_keys.get(&key) == Some(&1);
            if !unique {
                continue;
            }
            if let Some(matched) = previous
                .iter()
                .find(|p| !used.contains(&p.page_id) && p.url == *url && p.title == *title)
            {
                used.insert(matched.page_id.clone());
                slots[slot] = Some(PageBinding {
                    page_id: matched.page_id.clone(),
                    index: *index,
                    title: title.clone(),
                    url: url.clone(),
                    active: *active,
                });
            }
        }
        // Pass 2: remaining tabs reconcile deterministically by upstream index.
        for (slot, (index, active, title, url)) in tabs.iter().enumerate() {
            if slots[slot].is_some() {
                continue;
            }
            if let Some(matched) = previous
                .iter()
                .find(|p| !used.contains(&p.page_id) && p.index == *index)
            {
                used.insert(matched.page_id.clone());
                slots[slot] = Some(PageBinding {
                    page_id: matched.page_id.clone(),
                    index: *index,
                    title: title.clone(),
                    url: url.clone(),
                    active: *active,
                });
            }
        }
        // Pass 3: brand-new tabs get fresh opaque ids.
        for (slot, (index, active, title, url)) in tabs.iter().enumerate() {
            if slots[slot].is_none() {
                slots[slot] = Some(PageBinding {
                    page_id: self.new_page_id(),
                    index: *index,
                    title: title.clone(),
                    url: url.clone(),
                    active: *active,
                });
            }
        }
        let pages: Vec<PageBinding> = slots.into_iter().flatten().collect();
        self.set_pages(pages.clone()).await;
        pages
    }

    pub(crate) async fn page_index(&self, page_id: &str) -> Result<usize, CoreError> {
        self.page_bindings()
            .await
            .into_iter()
            .find(|p| p.page_id == page_id)
            .map(|p| p.index)
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::BrowserTargetNotFound,
                    "browser page id is not valid for the current session",
                )
            })
    }

    pub(crate) async fn active_launch(&self) -> Option<BrowserLaunchSpec> {
        self.state
            .lock()
            .await
            .active
            .as_ref()
            .map(|s| s.launch.clone())
    }

    /// Launch-time DevTools auto-open state for the active session, as reported
    /// by the driver that actually performed the launch.
    pub(crate) async fn devtools_auto_open(&self) -> Option<bool> {
        let state = self.state.lock().await;
        state
            .active
            .as_ref()
            .map(|s| self.driver.launch_devtools_auto_open(&s.launch))
    }

    pub(crate) async fn active_artifact_dir(&self) -> Result<PathBuf, CoreError> {
        self.guard_page_operation().await?;
        self.state
            .lock()
            .await
            .active
            .as_ref()
            .map(|s| s.launch.artifact_dir.clone())
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::BrowserNotRunning,
                    "browser is not running; call browser_open first",
                )
            })
    }

    pub(crate) async fn active_engine(&self) -> Option<BrowserEngine> {
        self.state
            .lock()
            .await
            .active
            .as_ref()
            .map(|s| s.driver.actual_engine)
    }

    pub(crate) async fn device_descriptor(&self, name: &str) -> Result<Value, CoreError> {
        self.guard_page_operation().await?;
        self.driver.device_descriptor(name).await
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

    #[derive(Default)]
    struct FakeDriver {
        starts: AtomicUsize,
        stops: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl BrowserDriver for FakeDriver {
        async fn start(&self, spec: BrowserLaunchSpec) -> Result<BrowserDriverStatus, CoreError> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Ok(BrowserDriverStatus {
                running: true,
                actual_engine: spec.engine,
                sidecar_version: Some("test".into()),
                browser_version: None,
                child_pid: None,
            })
        }

        async fn stop(&self) -> Result<(), CoreError> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn call(
            &self,
            _tool: &'static str,
            _arguments: Map<String, Value>,
        ) -> Result<CallToolResult, CoreError> {
            Ok(CallToolResult::success(vec![ContentBlock::text("ok")]))
        }

        async fn chromium_performance_metrics(&self) -> Result<Value, CoreError> {
            Ok(Value::Null)
        }
    }

    fn core(dir: &Path) -> Arc<CoreServices> {
        Arc::new(CoreServices::open(&dir.join("manager.db")).unwrap())
    }

    #[tokio::test]
    async fn disabled_and_pause_guards_have_deterministic_priority() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path());
        let driver = Arc::new(FakeDriver::default());
        let settings = AppSettings {
            browser_automation_enabled: false,
            ..Default::default()
        };
        let manager =
            BrowserManager::new(core.clone(), dir.path().into(), &settings, driver).unwrap();
        assert!(!manager.status().await.enabled);
        assert_eq!(
            manager.open(None).await.unwrap_err().code,
            ErrorCode::BrowserAutomationDisabled
        );
        core.pause.set(true);
        assert_eq!(
            manager.open(None).await.unwrap_err().code,
            ErrorCode::AccessPaused
        );
    }

    #[tokio::test]
    async fn disabling_stops_session_and_next_open_is_new() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path());
        let driver = Arc::new(FakeDriver::default());
        let settings = AppSettings::default();
        let manager =
            BrowserManager::new(core, dir.path().into(), &settings, driver.clone()).unwrap();

        let first = manager.open(None).await.unwrap();
        let first_id = first.session_id.unwrap();
        manager
            .apply_settings(BrowserRuntimeSettings { enabled: false })
            .await
            .unwrap();
        assert_eq!(driver.stops.load(Ordering::SeqCst), 1);
        assert!(!manager.status().await.enabled);

        manager
            .apply_settings(BrowserRuntimeSettings { enabled: true })
            .await
            .unwrap();
        let second = manager.open(None).await.unwrap();
        assert_ne!(second.session_id.as_deref(), Some(first_id.as_str()));
    }

    #[tokio::test]
    async fn pause_shutdown_stops_running_browser() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path());
        let driver = Arc::new(FakeDriver::default());
        let manager = BrowserManager::new(
            core.clone(),
            dir.path().into(),
            &AppSettings::default(),
            driver.clone(),
        )
        .unwrap();
        manager.open(None).await.unwrap();
        core.pause.set(true);
        manager.on_pause_changed(true).await.unwrap();
        assert_eq!(driver.stops.load(Ordering::SeqCst), 1);
        assert!(!manager.status().await.running);
    }

    #[tokio::test]
    async fn duplicate_tab_reconciliation_is_deterministic_and_never_swaps_ids() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path());
        let driver = Arc::new(FakeDriver::default());
        let manager =
            BrowserManager::new(core, dir.path().into(), &AppSettings::default(), driver).unwrap();
        manager.open(None).await.unwrap();

        let first = manager
            .reconcile_pages(vec![(0, true, "Dup".into(), "https://a.test/".into())])
            .await;
        assert_eq!(first.len(), 1);

        // Two identical tabs: matching by non-unique url/title is ambiguous,
        // so ids must follow the deterministic index rule instead.
        let both = manager
            .reconcile_pages(vec![
                (0, false, "Dup".into(), "https://a.test/".into()),
                (1, true, "Dup".into(), "https://a.test/".into()),
            ])
            .await;
        assert_eq!(both.len(), 2);
        assert_eq!(both[0].page_id, first[0].page_id, "index 0 keeps its id");
        assert_ne!(both[0].page_id, both[1].page_id);

        // Repeating the identical tab list never swaps the ids.
        let again = manager
            .reconcile_pages(vec![
                (0, false, "Dup".into(), "https://a.test/".into()),
                (1, true, "Dup".into(), "https://a.test/".into()),
            ])
            .await;
        assert_eq!(again[0].page_id, both[0].page_id);
        assert_eq!(again[1].page_id, both[1].page_id);

        // A unique url/title keeps its id across an index shift (earlier tab
        // closed upstream).
        let shifted = manager
            .reconcile_pages(vec![
                (0, false, "Dup".into(), "https://a.test/".into()),
                (1, true, "Other".into(), "https://b.test/".into()),
            ])
            .await;
        let other = manager
            .reconcile_pages(vec![(0, true, "Other".into(), "https://b.test/".into())])
            .await;
        assert_eq!(other[0].page_id, shifted[1].page_id);
    }
}
