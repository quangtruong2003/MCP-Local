use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::settings::{BrowserEngine, BrowserRunMode};

pub const DEFAULT_ACTION_TIMEOUT_MS: u64 = 10_000;
pub const DEFAULT_NAVIGATION_TIMEOUT_MS: u64 = 60_000;
pub const MAX_TOOL_TIMEOUT_MS: u64 = 120_000;
pub const MAX_VIEWPORT_EDGE: u32 = 8_192;
pub const MAX_BATCH_STEPS: usize = 20;
pub const DEFAULT_EVENT_LIMIT: usize = 100;
pub const MAX_EVENT_LIMIT: usize = 500;
pub const MAX_SNAPSHOT_BYTES: usize = 256 * 1024;
pub const MAX_COMPLETED_ARTIFACT_SESSIONS: usize = 10;
pub const MAX_ARTIFACT_AGE_DAYS: u64 = 7;
pub const MAX_ARTIFACT_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserRuntimeSettings {
    pub enabled: bool,
    pub engine: BrowserEngine,
    pub run_mode: BrowserRunMode,
}

impl From<&crate::settings::AppSettings> for BrowserRuntimeSettings {
    fn from(settings: &crate::settings::AppSettings) -> Self {
        Self {
            enabled: settings.browser_automation_enabled,
            engine: settings.browser_engine,
            run_mode: settings.browser_run_mode,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserEmulation {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: f64,
    pub is_mobile: bool,
    pub has_touch: bool,
    pub user_agent: Option<String>,
    pub landscape: bool,
}

#[derive(Debug, Clone)]
pub struct BrowserLaunchSpec {
    pub session_id: String,
    pub engine: BrowserEngine,
    pub run_mode: BrowserRunMode,
    pub profile_dir: PathBuf,
    pub artifact_dir: PathBuf,
    pub action_timeout_ms: u64,
    pub navigation_timeout_ms: u64,
    pub emulation: Option<BrowserEmulation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserPageStatus {
    pub page_id: String,
    pub url: String,
    pub title: String,
    pub viewport_width: Option<u32>,
    pub viewport_height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserSessionStatus {
    pub enabled: bool,
    pub paused: bool,
    pub configured_engine: BrowserEngine,
    pub actual_engine: Option<BrowserEngine>,
    pub run_mode: BrowserRunMode,
    pub running: bool,
    pub session_id: Option<String>,
    pub active_page: Option<BrowserPageStatus>,
    pub emulation: Option<BrowserEmulation>,
    pub runtime_version: Option<String>,
    pub browser_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserImage {
    pub png: Vec<u8>,
    pub original_width: u32,
    pub original_height: u32,
    pub returned_width: u32,
    pub returned_height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserArtifact {
    pub artifact_id: String,
    pub kind: String,
    pub bytes: u64,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserDownload {
    pub download_id: String,
    pub filename: String,
    pub bytes: u64,
    pub completed: bool,
    pub sha256: Option<String>,
}
