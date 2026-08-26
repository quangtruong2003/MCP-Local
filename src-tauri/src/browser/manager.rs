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

use lc_core::{error::{CoreError, ErrorCode}, CoreServices};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};
use tokio::sync::{Mutex, RwLock};

use crate::settings::{AppSettings, BrowserEngine};

use super::{
    artifacts::ArtifactStore,
    driver::{BrowserDriver, BrowserDriverStatus},
    playwright::PlaywrightMcpDriver,
    policy::validate_navigation_url,
    types::{
        BrowserEmulation, BrowserLaunchSpec, BrowserPageStatus, BrowserRuntimeSettings,
        BrowserSessionStatus, DEFAULT_ACTION_TIMEOUT_MS, DEFAULT_NAVIGATION_TIMEOUT_MS,
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

    fn engine_slug(engine: BrowserEngine) -> &'static str {
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
        format!(
            "p-{}",
            self.next_page.fetch_add(1, Ordering::Relaxed)
        )
    }

    async fn start_new_session(
        &self,
        settings: BrowserRuntimeSettings,
        emulation: Option<BrowserEmulation>,
    ) -> Result<BrowserSessionStatus, CoreError> {
        let session_id = self.new_session_id();
        let artifact_dir = self.artifacts.create_session(&session_id)?;
        let profile_dir = self.profile_dir_for(settings.engine);
        std::fs::create_dir_all(&profile_dir).map_err(CoreError::io)?;
        let launch = BrowserLaunchSpec {
            session_id: session_id.clone(),
            engine: settings.engine,
            run_mode: settings.run_mode,
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
                viewport_width: active
                    .and_then(|s| s.launch.emulation.as_ref().map(|e| e.width)),
                viewport_height: active
                    .and_then(|s| s.launch.emulation.as_ref().map(|e| e.height)),
            });
        BrowserSessionStatus {
            enabled: settings.enabled,
            paused: self.core.pause.is_paused(),
            configured_engine: settings.engine,
            actual_engine: active.map(|s| s.driver.actual_engine),
            run_mode: settings.run_mode,
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
        if running
            && (!settings.enabled
                || settings.engine != previous.engine
                || settings.run_mode != previous.run_mode)
        {
            // Settings were already committed in-memory above. Even if stop
            // fails, OFF/new config governs every subsequent guard.
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
            active.pages = pages
                .into_iter()
                .map(|p| (p.page_id.clone(), p))
                .collect();
            active.active_page_id = active
                .pages
                .values()
                .find(|p| p.active)
                .map(|p| p.page_id.clone());
        }
    }

    pub(crate) async fn page_bindings(&self) -> Vec<PageBinding> {
        let state = self.state.lock().await;
        state
            .active
            .as_ref()
            .map(|s| s.pages.values().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) async fn active_launch(&self) -> Option<BrowserLaunchSpec> {
        self.state.lock().await.active.as_ref().map(|s| s.launch.clone())
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

    use crate::settings::BrowserRunMode;
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
        let manager = BrowserManager::new(core.clone(), dir.path().into(), &settings, driver)
            .unwrap();
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
    async fn disabling_or_changing_launch_config_stops_session_and_next_open_is_new() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path());
        let driver = Arc::new(FakeDriver::default());
        let settings = AppSettings::default();
        let manager = BrowserManager::new(
            core,
            dir.path().into(),
            &settings,
            driver.clone(),
        )
        .unwrap();

        let first = manager.open(None).await.unwrap();
        let first_id = first.session_id.unwrap();
        manager
            .apply_settings(BrowserRuntimeSettings {
                enabled: true,
                engine: BrowserEngine::Firefox,
                run_mode: BrowserRunMode::Background,
            })
            .await
            .unwrap();
        assert_eq!(driver.stops.load(Ordering::SeqCst), 1);
        assert!(!manager.status().await.running);
        let second = manager.open(None).await.unwrap();
        assert_ne!(second.session_id.as_deref(), Some(first_id.as_str()));

        manager
            .apply_settings(BrowserRuntimeSettings {
                enabled: false,
                engine: BrowserEngine::Firefox,
                run_mode: BrowserRunMode::Background,
            })
            .await
            .unwrap();
        assert_eq!(driver.stops.load(Ordering::SeqCst), 2);
        assert!(!manager.status().await.enabled);
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
}
