use serde::{Deserialize, Serialize};

pub const DEFAULT_ACTION_TIMEOUT_MS: u64 = 10_000;
pub const DEFAULT_NAVIGATION_TIMEOUT_MS: u64 = 60_000;
pub const MAX_TOOL_TIMEOUT_MS: u64 = 120_000;
pub const MAX_VIEWPORT_EDGE: u32 = 8_192;
pub const MAX_BATCH_STEPS: usize = 20;
pub const DEFAULT_EVENT_LIMIT: usize = 100;
pub const MAX_EVENT_LIMIT: usize = 500;
pub const MAX_SNAPSHOT_BYTES: usize = 256 * 1024;
/// Screenshot resource bounds applied before decoding so a pathological image
/// cannot exhaust memory. Encoded-byte budget gates the base64 payload; the
/// pixel budget is checked against the PNG header before any full decode.
pub const DEFAULT_SCREENSHOT_MAX_EDGE: u32 = 2_048;
pub const MAX_SCREENSHOT_ENCODED_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_SCREENSHOT_PIXELS: u64 = 64 * 1024 * 1024;
pub const MAX_COMPLETED_ARTIFACT_SESSIONS: usize = 10;
pub const MAX_ARTIFACT_AGE_DAYS: u64 = 7;
pub const MAX_ARTIFACT_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

/// The fixed product-visible browser backend. v0.2.0 has exactly one backend:
/// Lightpanda running inside the app-owned WSL2 distro. This is a runtime
/// status identity, not a user choice.
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq,
)]
#[serde(rename_all = "snake_case")]
pub enum BrowserBackend {
    #[default]
    Lightpanda,
}

/// Legacy engine identity retained for the bundled Playwright runtime until
/// the Lightpanda migration removes it. No longer a user-visible setting.
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq,
)]
#[serde(rename_all = "snake_case")]
pub enum BrowserEngine {
    #[default]
    BundledChromium,
    Chrome,
    Edge,
    Firefox,
}

impl BrowserEngine {
    pub fn profile_key(self) -> &'static str {
        match self {
            Self::BundledChromium => "bundled_chromium",
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::Firefox => "firefox",
        }
    }

    pub fn is_chromium_family(self) -> bool {
        !matches!(self, Self::Firefox)
    }
}

/// Runtime-only run mode. Background is the sole supported mode; the legacy
/// visible-debug variant is retained until the Playwright runtime is removed.
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq,
)]
#[serde(rename_all = "snake_case")]
pub enum BrowserRunMode {
    #[default]
    Background,
    VisibleDebug,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserRuntimeSettings {
    pub enabled: bool,
}

impl From<&crate::settings::AppSettings> for BrowserRuntimeSettings {
    fn from(settings: &crate::settings::AppSettings) -> Self {
        Self {
            enabled: settings.browser_automation_enabled,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct BrowserEmulation {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: f64,
    pub is_mobile: bool,
    pub has_touch: bool,
    pub user_agent: Option<String>,
    pub landscape: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct BrowserPageStatus {
    pub page_id: String,
    pub url: String,
    pub title: String,
    pub viewport_width: Option<u32>,
    pub viewport_height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
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
