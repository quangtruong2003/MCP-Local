use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserOpenArgs {
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserTabAction {
    List,
    New,
    Select,
    Close,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserTabsArgs {
    pub action: BrowserTabAction,
    #[serde(default)]
    pub page_id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNavigationAction {
    Url,
    Back,
    Forward,
    Reload,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserNavigateArgs {
    pub action: BrowserNavigationAction,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserSearchArgs {
    pub query: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserTargetArgs {
    /// Element target reference from the current browser snapshot.
    #[serde(rename = "ref")]
    pub ref_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserFillArgs {
    #[serde(rename = "ref")]
    pub ref_id: String,
    pub text: String,
    #[serde(default)]
    pub submit: bool,
    #[serde(default)]
    pub slowly: bool,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserPressArgs {
    pub key: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserSelectArgs {
    #[serde(rename = "ref")]
    pub ref_id: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserDragArgs {
    pub start_ref: String,
    pub end_ref: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLoadState {
    Loading,
    Interactive,
    Complete,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserWaitCondition {
    TextPresent { text: String },
    TextAbsent { text: String },
    RefVisible { ref_id: String },
    RefHidden { ref_id: String },
    UrlContains { value: String },
    LoadState { state: BrowserLoadState },
    Delay { milliseconds: u64 },
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserWaitArgs {
    pub condition: BrowserWaitCondition,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserDialogArgs {
    pub accept: bool,
    #[serde(default)]
    pub prompt_text: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserViewportArgs {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserOrientation {
    Portrait,
    Landscape,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum BrowserEmulationRequest {
    Preset {
        name: String,
        #[serde(default)]
        orientation: Option<BrowserOrientation>,
    },
    Explicit {
        width: u32,
        height: u32,
        device_scale_factor: f64,
        is_mobile: bool,
        has_touch: bool,
        user_agent: String,
        orientation: BrowserOrientation,
    },
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserEmulateArgs {
    pub emulation: BrowserEmulationRequest,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserScreenshotMode {
    Viewport,
    FullPage,
    Element,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserScreenshotArgs {
    #[serde(default = "default_screenshot_mode")]
    pub mode: BrowserScreenshotMode,
    /// Element target reference; required for `element` mode and rejected otherwise.
    #[serde(default, rename = "ref")]
    pub ref_id: Option<String>,
    #[serde(default)]
    pub max_edge: Option<u32>,
}

fn default_screenshot_mode() -> BrowserScreenshotMode {
    BrowserScreenshotMode::Viewport
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserConsoleArgs {
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    /// Opaque pagination cursor returned by a previous `browser_console` call.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserNetworkArgs {
    /// Regular expression matched against request URLs by upstream Playwright MCP
    /// (e.g. "/api/.*user"). Forwarded as-is to the upstream `filter` argument;
    /// must be at most 4096 bytes and contain no control characters.
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub include_static: bool,
    #[serde(default)]
    pub limit: Option<usize>,
    /// Opaque pagination cursor returned by a previous `browser_network` call.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserDomArgs {
    #[serde(rename = "ref")]
    pub ref_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserEvaluateArgs {
    pub expression: String,
    #[serde(default, rename = "ref")]
    pub ref_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserTraceAction {
    Start,
    Stop,
    Status,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserTraceArgs {
    pub action: BrowserTraceAction,
}

/// One upload source. Closed tagged enum: the caller must explicitly declare
/// whether the file comes from an approved workspace path or from an opaque
/// active-session browser artifact id. No other source kind is representable.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserUploadSource {
    /// Workspace-relative path; requires workspace Read permission.
    Workspace { path: String },
    /// Opaque artifact id that must resolve inside the active browser session
    /// artifact sandbox (e.g. a completed download).
    Artifact { artifact_id: String },
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserUploadArgs {
    /// Workspace used for `workspace` sources; required when any source is a
    /// workspace path.
    #[serde(default)]
    pub workspace_id: Option<String>,
    pub sources: Vec<BrowserUploadSource>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserSaveDownloadArgs {
    pub workspace_id: String,
    pub download_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserStep {
    Navigate {
        url: String,
    },
    Snapshot,
    Click {
        ref_id: String,
    },
    Fill {
        ref_id: String,
        text: String,
    },
    Press {
        key: String,
    },
    Select {
        ref_id: String,
        values: Vec<String>,
    },
    Wait {
        condition: BrowserWaitCondition,
        timeout_ms: Option<u64>,
    },
    SetViewport {
        width: u32,
        height: u32,
    },
    Screenshot {
        mode: BrowserScreenshotMode,
        #[serde(default)]
        ref_id: Option<String>,
        #[serde(default)]
        max_edge: Option<u32>,
    },
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserRunStepsArgs {
    pub steps: Vec<BrowserStep>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserPageSummary {
    pub page_id: String,
    pub title: String,
    pub url: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserScreenshotMetadata {
    pub mode: BrowserScreenshotMode,
    pub original_width: u32,
    pub original_height: u32,
    pub returned_width: u32,
    pub returned_height: u32,
    pub encoded_bytes: usize,
    pub sha256: String,
}

/// Typed success shapes behind every browser tool's `outputSchema`. Each
/// struct serializes exactly to the `structuredContent` the handler returns,
/// so clients that validate structured output see one stable contract.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserOkOutput {
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserTextOutput {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserTextTruncatedOutput {
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserOpenOutput {
    pub status: super::types::BrowserSessionStatus,
    pub pages: Vec<BrowserPageSummary>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserNavigationOutput {
    pub text: String,
    pub truncated: bool,
    pub pages: Vec<BrowserPageSummary>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserSnapshotOutput {
    pub snapshot: String,
    pub bytes: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserViewportOutput {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserConsoleOutput {
    pub level: String,
    pub count: usize,
    pub levels: std::collections::BTreeMap<String, u64>,
    pub messages: String,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserNetworkOutput {
    pub count: usize,
    pub requests: String,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserDomOutput {
    pub target: String,
    pub result: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserEvaluateOutput {
    pub result: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserPerformanceOutput {
    pub page: String,
    pub page_truncated: bool,
    pub cdp_supported: bool,
    pub cdp: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserTraceOutput {
    pub tracing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserDevtoolsOutput {
    pub supported: bool,
    pub mode: String,
    pub engine: super::types::BrowserEngine,
    pub auto_open: bool,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserUploadOutput {
    pub file_count: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserStepError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserRunStepsOutput {
    pub ok: bool,
    pub results: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BrowserStepError>,
}

pub(crate) fn call_result_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| content.as_text().map(|text| text.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn truncate_utf8(mut text: String, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut boundary = max_bytes;
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    text.truncate(boundary);
    (text, true)
}
