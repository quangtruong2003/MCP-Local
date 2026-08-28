use lc_core::error::{CoreError, ErrorCode};
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
pub struct BrowserMarkdownArgs {
    /// Optional element ref from the current snapshot to scope the render.
    #[serde(default, rename = "ref")]
    pub ref_id: Option<String>,
    /// Optional soft cap on returned bytes, bounded by the server.
    #[serde(default)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserHtmlArgs {
    /// Optional element ref from the current snapshot to scope the dump.
    #[serde(default, rename = "ref")]
    pub ref_id: Option<String>,
    /// Optional soft cap on returned bytes, bounded by the server.
    #[serde(default)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserFindElementArgs {
    /// ARIA role to match, such as button, link, or textbox.
    #[serde(default)]
    pub role: Option<String>,
    /// Accessible name substring to match.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserExtractArgs {
    /// Extraction schema: a JSON object mapping output field names to
    /// CSS-selector specs. Validated and rejected before any native call.
    pub schema: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserScrollArgs {
    /// Optional element ref from the current snapshot to scroll instead of
    /// the window.
    #[serde(default, rename = "ref")]
    pub ref_id: Option<String>,
    /// Horizontal scroll offset.
    #[serde(default)]
    pub x: Option<i64>,
    /// Vertical scroll offset.
    #[serde(default)]
    pub y: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BrowserSetCheckedArgs {
    #[serde(rename = "ref")]
    pub ref_id: String,
    /// Check (true) or uncheck (false). Defaults to true.
    #[serde(default = "default_checked_true")]
    pub checked: bool,
}

fn default_checked_true() -> bool {
    true
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
pub struct BrowserExtractOutput {
    /// Parsed structured result from the extraction schema.
    pub data: serde_json::Value,
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
    /// Explicit coverage marker: the Lightpanda runtime exposes only
    /// Performance Resource Timing, so method/status/headers are unavailable.
    pub coverage: String,
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
    /// The timing backend that produced this data. Always `lightpanda` here.
    pub backend: String,
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

// ---------------------------------------------------------------------------
// Native Lightpanda 31-tool public argument contracts.
//
// These structs mirror the pinned Lightpanda 0.3.7 upstream inputSchemas
// field-for-field (names, types, required sets, enums, defaults). The only
// systematic Local Workspaces addition is the optional `session_id` routing
// metadata on every tool except the three session-control tools; it is
// stripped before forwarding and transported as the `Mcp-Session-Id` header.
// The schema parity test in `schema_tests` is authoritative: public schema
// minus `session_id` must equal the pinned native schema.
// ---------------------------------------------------------------------------

/// Native Lightpanda load/wait states, exactly as advertised upstream.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LightpandaLoadState {
    Load,
    DomContentLoaded,
    NetworkAlmostIdle,
    NetworkIdle,
    Done,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaGotoArgs {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, rename = "waitUntil", skip_serializing_if = "Option::is_none")]
    pub wait_until: Option<LightpandaLoadState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaSearchArgs {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaMarkdownArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, rename = "maxBytes", skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaHtmlArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaLinksArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaEvaluateArgs {
    pub script: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaExtractArgs {
    pub schema: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaTreeArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, rename = "maxDepth", skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaNodeDetailsArgs {
    #[serde(rename = "backendNodeId")]
    pub backend_node_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaInteractiveElementsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaStructuredDataArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaDetectFormsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaClickArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaFillArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaScrollArgs {
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaWaitForSelectorArgs {
    pub selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaWaitForScriptArgs {
    pub script: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaWaitForStateArgs {
    pub state: LightpandaLoadState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaHoverArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaPressArgs {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaSelectOptionArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// `setChecked` preserves the unusual upstream contract exactly: `checked` is
/// advertised as required AND as defaulting to true. schemars' derive cannot
/// express both at once (a serde default makes the field optional in the
/// generated schema), so the schema is emitted manually and the parity test
/// is authoritative.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LightpandaSetCheckedArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default = "default_checked_true")]
    pub checked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl schemars::JsonSchema for LightpandaSetCheckedArgs {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LightpandaSetCheckedArgs".into()
    }

    fn json_schema(_generator: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        let mut properties = serde_json::Map::new();
        properties.insert(
            "selector".into(),
            serde_json::json!({"type": ["string", "null"]}),
        );
        properties.insert(
            "backendNodeId".into(),
            serde_json::json!({"type": ["integer", "null"], "format": "int64"}),
        );
        properties.insert(
            "checked".into(),
            serde_json::json!({"type": "boolean", "default": true}),
        );
        properties.insert(
            "session_id".into(),
            serde_json::json!({"type": ["string", "null"]}),
        );
        let mut schema = serde_json::Map::new();
        schema.insert("type".into(), serde_json::json!("object"));
        schema.insert("properties".into(), serde_json::Value::Object(properties));
        schema.insert("required".into(), serde_json::json!(["checked"]));
        schema.into()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaFindElementArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaConsoleLogsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaGetUrlArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaGetCookiesArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub all: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaGetEnvArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaSaveArgs {
    pub path: String,
    pub script: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaSessionNewArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaSessionListArgs {}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaSessionCloseArgs {
    pub id: String,
}

/// Splits typed public arguments into the Local Workspaces routing metadata
/// (`session_id`) and the untouched native argument map. Nothing else is
/// converted: native field names, enum values, and payload strings cross
/// unchanged.
pub fn split_session_route<T: Serialize>(
    args: &T,
) -> Result<(Option<String>, serde_json::Map<String, serde_json::Value>), CoreError> {
    let mut obj = serde_json::to_value(args)
        .map_err(|_| CoreError::new(ErrorCode::InvalidArgument, "invalid browser arguments"))?
        .as_object()
        .cloned()
        .ok_or_else(|| {
            CoreError::new(
                ErrorCode::InvalidArgument,
                "browser arguments must be an object",
            )
        })?;
    let session_id = obj
        .remove("session_id")
        .and_then(|v| v.as_str().map(str::to_string));
    Ok((session_id, obj))
}

/// Applies the Local Workspaces navigation URL policy to the `url` argument
/// of native tools whose URL actually causes navigation. `getCookies.url`
/// only scopes cookie output and is deliberately not gated here.
pub fn validate_native_navigation(
    tool: &str,
    args: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), CoreError> {
    if !super::policy::NATIVE_NAVIGATION_URL_TOOLS.contains(&tool) {
        return Ok(());
    }
    let Some(url) = args.get("url").and_then(serde_json::Value::as_str) else {
        return Ok(());
    };
    super::policy::validate_navigation_url(url)?;
    Ok(())
}

/// Builds content-free audit metadata for one native tool call. Records only
/// safe facts: origins (never full URLs), byte lengths, presence flags,
/// booleans, integers, and fixed enum values. Never records raw queries,
/// fill values, scripts, extraction schemas, cookie/env results, page or
/// console content, save paths/scripts, or raw session ids/names.
pub fn safe_native_audit_detail(
    tool: &str,
    args: &serde_json::Map<String, serde_json::Value>,
    session_id: Option<&str>,
) -> serde_json::Value {
    use serde_json::{Map, Value};

    fn byte_len(detail: &mut Map<String, Value>, key: &str, args: &Map<String, Value>, arg: &str) {
        if let Some(value) = args.get(arg).and_then(Value::as_str) {
            detail.insert(key.into(), Value::Number(value.len().into()));
        }
    }
    fn present(detail: &mut Map<String, Value>, key: &str, args: &Map<String, Value>, arg: &str) {
        detail.insert(key.into(), Value::Bool(args.contains_key(arg)));
    }
    fn copy_bool(detail: &mut Map<String, Value>, key: &str, args: &Map<String, Value>, arg: &str) {
        if let Some(value) = args.get(arg).and_then(Value::as_bool) {
            detail.insert(key.into(), Value::Bool(value));
        }
    }
    fn copy_int(detail: &mut Map<String, Value>, key: &str, args: &Map<String, Value>, arg: &str) {
        if let Some(value) = args.get(arg).and_then(Value::as_i64) {
            detail.insert(key.into(), Value::Number(value.into()));
        }
    }
    fn copy_enum(detail: &mut Map<String, Value>, key: &str, args: &Map<String, Value>, arg: &str) {
        if let Some(value) = args.get(arg).and_then(Value::as_str) {
            detail.insert(key.into(), Value::String(value.to_string()));
        }
    }
    fn origin(detail: &mut Map<String, Value>, args: &Map<String, Value>) {
        if let Some(url) = args.get("url").and_then(Value::as_str) {
            if let Ok(origin) = super::policy::redacted_origin(url) {
                detail.insert("origin".into(), Value::String(origin));
            }
        }
    }
    fn target_kind(args: &Map<String, Value>) -> &'static str {
        if args.contains_key("backendNodeId") {
            "backend_node_id"
        } else if args.contains_key("selector") {
            "selector"
        } else {
            "none"
        }
    }

    let mut detail = Map::new();
    match tool {
        "goto" => {
            origin(&mut detail, args);
            copy_enum(&mut detail, "wait_until", args, "waitUntil");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "search" => byte_len(&mut detail, "query_bytes", args, "query"),
        "markdown" => {
            origin(&mut detail, args);
            present(&mut detail, "selector_present", args, "selector");
            present(
                &mut detail,
                "backend_node_id_present",
                args,
                "backendNodeId",
            );
            copy_int(&mut detail, "max_bytes", args, "maxBytes");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "html" => {
            origin(&mut detail, args);
            present(&mut detail, "selector_present", args, "selector");
            present(
                &mut detail,
                "backend_node_id_present",
                args,
                "backendNodeId",
            );
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "links" | "interactiveElements" | "structuredData" | "detectForms" => {
            origin(&mut detail, args);
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "tree" => {
            origin(&mut detail, args);
            present(
                &mut detail,
                "backend_node_id_present",
                args,
                "backendNodeId",
            );
            copy_int(&mut detail, "max_depth", args, "maxDepth");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "nodeDetails" => copy_int(&mut detail, "backend_node_id", args, "backendNodeId"),
        "evaluate" => {
            byte_len(&mut detail, "script_bytes", args, "script");
            origin(&mut detail, args);
            present(&mut detail, "save_present", args, "save");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "extract" => {
            byte_len(&mut detail, "schema_bytes", args, "schema");
            present(&mut detail, "save_present", args, "save");
        }
        "click" | "hover" => {
            detail.insert("target".into(), serde_json::json!(target_kind(args)));
        }
        "fill" | "selectOption" => {
            detail.insert("target".into(), serde_json::json!(target_kind(args)));
            byte_len(&mut detail, "value_bytes", args, "value");
        }
        "scroll" => {
            present(
                &mut detail,
                "backend_node_id_present",
                args,
                "backendNodeId",
            );
            present(&mut detail, "x_present", args, "x");
            present(&mut detail, "y_present", args, "y");
        }
        "waitForSelector" => {
            byte_len(&mut detail, "selector_bytes", args, "selector");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "waitForScript" => {
            byte_len(&mut detail, "script_bytes", args, "script");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "waitForState" => {
            copy_enum(&mut detail, "state", args, "state");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "press" => {
            copy_enum(&mut detail, "key", args, "key");
            detail.insert("target".into(), serde_json::json!(target_kind(args)));
        }
        "setChecked" => {
            detail.insert("target".into(), serde_json::json!(target_kind(args)));
            copy_bool(&mut detail, "checked", args, "checked");
        }
        "findElement" => {
            present(&mut detail, "role_present", args, "role");
            present(&mut detail, "name_present", args, "name");
        }
        "getCookies" => {
            copy_bool(&mut detail, "all", args, "all");
            origin(&mut detail, args);
        }
        "getEnv" => present(&mut detail, "name_present", args, "name"),
        "save" => {
            byte_len(&mut detail, "path_bytes", args, "path");
            byte_len(&mut detail, "script_bytes", args, "script");
        }
        "session_new" => present(&mut detail, "name_present", args, "name"),
        "session_close" => byte_len(&mut detail, "id_bytes", args, "id"),
        // consoleLogs, getUrl, session_list: session fingerprint only.
        _ => {}
    }
    detail.insert("session_present".into(), Value::Bool(session_id.is_some()));
    if let Some(id) = session_id {
        detail.insert("session_bytes".into(), Value::Number(id.len().into()));
    }
    Value::Object(detail)
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

// ---------------------------------------------------------------------------
// Opaque ref adapters. Lightpanda backend node ids never become the durable
// public identity: every backend id that crosses into model-facing output is
// replaced by an app-generated ref scoped to (page_id, generation).
// ---------------------------------------------------------------------------

/// True when a string is a well-formed opaque element ref (`e1`, `e2`, ...).
pub(crate) fn is_valid_ref(ref_id: &str) -> bool {
    match ref_id.strip_prefix('e') {
        Some(digits) => !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// Extracts the CSS selector from a Lightpanda `nodeDetails` JSON document.
/// Returns `None` for unparsable documents or missing/blank selectors.
pub(crate) fn parse_node_selector(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let selector = value.get("selector")?.as_str()?.trim();
    if selector.is_empty() {
        None
    } else {
        Some(selector.to_string())
    }
}

/// Replaces the leading backend node id in each Lightpanda `tree` line with an
/// opaque ref assigned by `register`. Tree lines are `<indent><id> <rest>`;
/// anything after the id is preserved verbatim, and lines without a leading
/// id pass through untouched.
pub(crate) fn normalize_tree_text(text: &str, register: &mut dyn FnMut(u64) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut first = true;
    for line in text.split('\n') {
        if !first {
            out.push('\n');
        }
        first = false;
        let indent = line.len() - line.trim_start_matches(' ').len();
        let rest = &line[indent..];
        let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digits > 0 {
            match rest[..digits].parse::<u64>() {
                Ok(id) => {
                    out.push_str(&line[..indent]);
                    out.push_str(&register(id));
                    out.push_str(&rest[digits..]);
                }
                // Id larger than u64: never leak it, leave the line untouched.
                Err(_) => out.push_str(line),
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Recursively replaces integer `backendNodeId` fields with opaque `ref`
/// string fields in one Lightpanda JSON document (links / interactiveElements
/// / findElement / detectForms). Raw backend ids are never preserved: a
/// malformed id field is dropped rather than leaked.
pub(crate) fn normalize_backend_node_ids(
    value: serde_json::Value,
    register: &mut dyn FnMut(u64) -> String,
) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, val) in map {
                if key == "backendNodeId" {
                    if let Some(id) = val.as_u64() {
                        out.insert("ref".into(), serde_json::Value::String(register(id)));
                    }
                } else {
                    out.insert(key, normalize_backend_node_ids(val, register));
                }
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .map(|item| normalize_backend_node_ids(item, register))
                .collect(),
        ),
        other => other,
    }
}

/// Defense-in-depth redaction for `detectForms` output. Upstream Lightpanda
/// already reports password input values as `*****`, but this pass guarantees
/// a password field's value never leaves the manager even if a future
/// upstream release changes that behavior. Any object carrying
/// `inputType: "password"` has its `value` forced to `*****`.
pub(crate) fn redact_form_passwords(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let is_password = map
                .get("inputType")
                .and_then(|v| v.as_str())
                .is_some_and(|t| t.eq_ignore_ascii_case("password"));
            if is_password && map.contains_key("value") {
                map.insert(
                    "value".into(),
                    serde_json::Value::String("*****".to_string()),
                );
            }
            for val in map.values_mut() {
                redact_form_passwords(val);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_form_passwords(item);
            }
        }
        _ => {}
    }
}

/// Hard cap applied to native-first content output when the caller supplies
/// no `max_bytes`, and the largest `max_bytes` value accepted.
pub(crate) const MAX_CONTENT_BYTES: usize = super::types::MAX_SNAPSHOT_BYTES;
/// Maximum encoded size of one `browser_extract` schema document.
pub(crate) const MAX_EXTRACT_SCHEMA_BYTES: usize = 64 * 1024;
/// Maximum nesting depth of one `browser_extract` schema document.
pub(crate) const MAX_EXTRACT_SCHEMA_DEPTH: usize = 8;
/// Maximum number of selector-like string entries in one extract schema.
pub(crate) const MAX_EXTRACT_SCHEMA_STRINGS: usize = 256;
/// Maximum `limit` value accepted inside an extract schema array spec.
pub(crate) const MAX_EXTRACT_LIMIT: u64 = 10_000;
/// Largest scroll offset accepted on either axis.
pub(crate) const MAX_SCROLL_OFFSET: i64 = 10_000_000;

/// Resolves the caller-supplied content cap, bounded to MAX_CONTENT_BYTES.
pub(crate) fn validated_content_cap(max_bytes: Option<u64>) -> Result<usize, CoreError> {
    match max_bytes {
        None => Ok(MAX_CONTENT_BYTES),
        Some(n) if n >= 1 && n <= MAX_CONTENT_BYTES as u64 => Ok(n as usize),
        Some(_) => Err(CoreError::new(
            ErrorCode::InvalidArgument,
            format!("max_bytes must be between 1 and {MAX_CONTENT_BYTES}"),
        )),
    }
}

/// Requires at least one bounded search criterion for find-element.
pub(crate) fn validate_find_element(
    role: Option<&str>,
    name: Option<&str>,
) -> Result<(), CoreError> {
    let role = role.unwrap_or_default().trim();
    let name = name.unwrap_or_default().trim();
    if role.is_empty() && name.is_empty() {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "browser_find_element requires role, name, or both",
        ));
    }
    if role.len() > 128 {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "browser_find_element role must be at most 128 bytes",
        ));
    }
    if name.len() > 512 {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "browser_find_element name must be at most 512 bytes",
        ));
    }
    Ok(())
}

/// Validates scroll offsets before any native call.
pub(crate) fn validate_scroll_offsets(x: Option<i64>, y: Option<i64>) -> Result<(), CoreError> {
    let in_bounds = |v: Option<i64>| v.is_none_or(|v| v.abs() <= MAX_SCROLL_OFFSET);
    if in_bounds(x) && in_bounds(y) {
        Ok(())
    } else {
        Err(CoreError::new(
            ErrorCode::InvalidArgument,
            format!("scroll offsets must be within +/-{MAX_SCROLL_OFFSET}"),
        ))
    }
}

/// Encodes one string as a JSON/JS string literal so page data is embedded as
/// data, never concatenated as executable source.
pub(crate) fn encode_js_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
}

/// Adapts a public `browser_evaluate` expression to Lightpanda's `evaluate`
/// script contract. Playwright-style function literals are invoked once so
/// they return their value; with an element selector the recovered element is
/// passed as the first argument.
pub(crate) fn page_evaluate_script(expression: &str, selector: Option<&str>) -> String {
    let trimmed = expression.trim();
    let is_function_literal = trimmed.starts_with("()")
        || trimmed.starts_with("async")
        || trimmed.starts_with("function")
        || (trimmed.starts_with('(') && trimmed.contains("=>"));
    match selector {
        Some(selector) => {
            let target = format!("document.querySelector({})", encode_js_string(selector));
            if is_function_literal {
                format!("({trimmed})({target})")
            } else {
                format!("((el) => ({trimmed}))({target})")
            }
        }
        None => {
            if is_function_literal {
                format!("({trimmed})()")
            } else {
                trimmed.to_string()
            }
        }
    }
}

/// Fixed app-owned page-context expressions for the truthful compatibility
/// adapters (Task 11). Each runs through the native `evaluate` tool and never
/// through process-level code execution. They are named constants so tests can
/// prove model/MCP input is never interpolated into them.
///
/// Navigation/Resource Performance APIs only. The Lightpanda runtime exposes
/// no CDP, so no renderer/CDP metrics are collected here.
pub(crate) const PERFORMANCE_TIMING_SCRIPT: &str = "() => ({navigation: performance.getEntriesByType('navigation').map(e=>({name:e.name,duration:e.duration,domContentLoadedEventEnd:e.domContentLoadedEventEnd,loadEventEnd:e.loadEventEnd})), resources: performance.getEntriesByType('resource').slice(-100).map(e=>({name:e.name,initiatorType:e.initiatorType,duration:e.duration,transferSize:e.transferSize}))})";

/// Performance Resource Timing is the only network signal the Lightpanda
/// runtime exposes; method/status/headers are not available and are never
/// fabricated.
pub(crate) const NETWORK_RESOURCE_TIMING_SCRIPT: &str = "() => ({resources: performance.getEntriesByType('resource').slice(-200).map(e=>({name:e.name,initiatorType:e.initiatorType,duration:e.duration,transferSize:e.transferSize}))})";

/// Element outerHTML + computed-style allowlist for `browser_dom`. Sensitive
/// attributes and password values are redacted on a CLONED node so the live
/// DOM is never mutated. No layout rect is read: Lightpanda performs no
/// layout, so the public rect stays null rather than fabricated.
pub(crate) const DOM_OUTER_SCRIPT: &str = r#"(element) => {
  if (!element) return null;
  const sensitive = (name) => {
    const n = String(name || '').toLowerCase();
    return n.includes('token') || n.includes('secret') || n.includes('password') || n === 'value';
  };
  const redact = (node) => {
    for (const a of Array.from(node.attributes || [])) {
      if (sensitive(a.name)) { try { node.setAttribute(a.name, '[redacted]'); } catch (e) {} }
    }
    const tag = String(node.tagName || '').toUpperCase();
    const type = String((node.getAttribute && node.getAttribute('type')) || '').toLowerCase();
    if (tag === 'INPUT' && type === 'password') { try { node.setAttribute('value', '[redacted]'); } catch (e) {} }
  };
  const clone = element.cloneNode(true);
  redact(clone);
  if (clone.querySelectorAll) { for (const d of clone.querySelectorAll('*')) redact(d); }
  let style = null;
  try {
    const cs = getComputedStyle(element);
    const allow = ['display','visibility','position','color','backgroundColor','fontSize','opacity','cursor'];
    style = {};
    for (const p of allow) { if (cs[p] !== undefined) style[p] = cs[p]; }
  } catch (e) { style = null; }
  return { outerHTML: String(clone.outerHTML || '').slice(0, 32768), style: style };
}"#;

/// Synthetic HTML5 drag between two recovered selectors. Uses a DataTransfer
/// and dispatched DragEvents only; there is no coordinate API under Lightpanda.
/// Returns false (surfaced as unsupported) when the runtime lacks the DragEvent
/// / DataTransfer constructors or either element is missing.
pub(crate) const DRAG_SYNTHETIC_SCRIPT: &str = r#"(startSel, endSel) => {
  const start = document.querySelector(startSel);
  const end = document.querySelector(endSel);
  if (!start || !end) return false;
  if (typeof DataTransfer === 'undefined' || typeof DragEvent === 'undefined') return false;
  const dt = new DataTransfer();
  const fire = (type, target) => {
    try { target.dispatchEvent(new DragEvent(type, { dataTransfer: dt, bubbles: true, cancelable: true })); } catch (e) {}
  };
  fire('dragstart', start);
  fire('dragenter', end);
  fire('dragover', end);
  fire('drop', end);
  fire('dragend', start);
  return true;
}"#;

/// Builds the concrete drag script by binding the two recovered selectors as
/// JSON string literals. Selectors are data, never concatenated as code.
pub(crate) fn drag_synthetic_script(start_selector: &str, end_selector: &str) -> String {
    format!(
        "({DRAG_SYNTHETIC_SCRIPT})({},{})",
        encode_js_string(start_selector),
        encode_js_string(end_selector)
    )
}

/// Synthetic hover for runtimes without a native hover primitive: dispatches
/// the mouseover/mouseenter/mousemove sequence on the recovered element.
/// Returns false when the element is missing.
pub(crate) const HOVER_SYNTHETIC_SCRIPT: &str = r#"(sel) => {
  const el = document.querySelector(sel);
  if (!el) return false;
  for (const type of ['mouseover', 'mouseenter', 'mousemove']) {
    try { el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true })); } catch (e) {}
  }
  return true;
}"#;

/// Builds the concrete hover script by binding the recovered selector as a
/// JSON string literal. The selector is data, never concatenated as code.
pub(crate) fn hover_synthetic_script(selector: &str) -> String {
    format!("({HOVER_SYNTHETIC_SCRIPT})({})", encode_js_string(selector))
}

/// Initiator types treated as static subresources for the `include_static`
/// network filter. Anything else (fetch/xhr/beacon/...) is dynamic.
pub(crate) fn is_static_initiator(initiator_type: &str) -> bool {
    matches!(
        initiator_type,
        "img"
            | "image"
            | "script"
            | "link"
            | "css"
            | "font"
            | "iframe"
            | "media"
            | "object"
            | "embed"
            | "svg"
    )
}

/// Redacts sensitive query-parameter values in one URL. Only the value of a
/// key that looks credential-like is replaced; the key and the rest of the URL
/// stay intact so filtering/auditing remains useful.
pub(crate) fn redact_sensitive_url(url: &str) -> String {
    const SENSITIVE: &[&str] = &[
        "token",
        "access_token",
        "api_key",
        "apikey",
        "key",
        "secret",
        "password",
        "passwd",
        "session",
        "sessionid",
        "auth",
        "authorization",
        "sig",
        "signature",
    ];
    let Some(query_pos) = url.find('?') else {
        return url.to_string();
    };
    let (head, query) = url.split_at(query_pos + 1);
    let redacted: Vec<String> = query
        .split('&')
        .map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next().unwrap_or_default();
            let value = parts.next();
            let key_lower = key.to_ascii_lowercase();
            if value.is_some() && SENSITIVE.iter().any(|s| key_lower.contains(s)) {
                format!("{key}=[redacted]")
            } else {
                pair.to_string()
            }
        })
        .collect();
    format!("{head}{}", redacted.join("&"))
}

/// Validates one `browser_extract` schema before any native call and returns
/// its canonical encoded form for the upstream `extract` tool. Rejects
/// non-object roots, oversized documents, deep nesting, too many selector
/// strings, and malformed or excessive `limit` values.
pub(crate) fn validate_extract_schema(schema: &serde_json::Value) -> Result<String, CoreError> {
    if !schema.is_object() {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "browser_extract schema must be a JSON object",
        ));
    }
    let encoded = serde_json::to_string(schema).map_err(|_| {
        CoreError::new(
            ErrorCode::InvalidArgument,
            "browser_extract schema is not serializable",
        )
    })?;
    if encoded.len() > MAX_EXTRACT_SCHEMA_BYTES {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            format!("browser_extract schema must be at most {MAX_EXTRACT_SCHEMA_BYTES} bytes"),
        ));
    }
    let mut strings = 0usize;
    walk_extract_schema(schema, 1, &mut strings)?;
    Ok(encoded)
}

fn walk_extract_schema(
    value: &serde_json::Value,
    depth: usize,
    strings: &mut usize,
) -> Result<(), CoreError> {
    if depth > MAX_EXTRACT_SCHEMA_DEPTH {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            format!("browser_extract schema nesting exceeds {MAX_EXTRACT_SCHEMA_DEPTH} levels"),
        ));
    }
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                if key == "limit" {
                    match val.as_u64() {
                        Some(n) if n <= MAX_EXTRACT_LIMIT => {}
                        _ => {
                            return Err(CoreError::new(
                                ErrorCode::InvalidArgument,
                                format!(
                                    "browser_extract limit must be an integer between 0 and {MAX_EXTRACT_LIMIT}"
                                ),
                            ))
                        }
                    }
                }
                walk_extract_schema(val, depth + 1, strings)?;
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk_extract_schema(item, depth + 1, strings)?;
            }
        }
        serde_json::Value::String(_) => {
            *strings += 1;
            if *strings > MAX_EXTRACT_SCHEMA_STRINGS {
                return Err(CoreError::new(
                    ErrorCode::InvalidArgument,
                    format!(
                        "browser_extract schema has more than {MAX_EXTRACT_SCHEMA_STRINGS} selector entries"
                    ),
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod schema_tests {
    //! Schema parity: every public `browser_lightpanda_*` argument schema,
    //! minus the Local Workspaces-only `session_id`, must equal the pinned
    //! Lightpanda 0.3.7 upstream inputSchema (property names/types, required
    //! sets, enum arrays, defaults).

    use super::*;
    use schemars::generate::SchemaSettings;
    use serde_json::Value;
    use std::collections::BTreeSet;

    fn public_schema<T: schemars::JsonSchema>() -> Value {
        // Same settings rmcp uses when publishing tool inputSchemas.
        let generator = SchemaSettings::draft2020_12().into_generator();
        let schema = generator.into_root_schema_for::<T>();
        serde_json::to_value(schema).unwrap()
    }

    /// One entry per pinned native tool, keyed by the upstream name, in
    /// contract mapping order. A missing public args type fails the lookup.
    fn native_public_arg_schemas() -> Vec<(&'static str, Value)> {
        let mut schemas: std::collections::HashMap<&'static str, Value> =
            std::collections::HashMap::from([
                ("goto", public_schema::<LightpandaGotoArgs>()),
                ("search", public_schema::<LightpandaSearchArgs>()),
                ("markdown", public_schema::<LightpandaMarkdownArgs>()),
                ("html", public_schema::<LightpandaHtmlArgs>()),
                ("links", public_schema::<LightpandaLinksArgs>()),
                ("evaluate", public_schema::<LightpandaEvaluateArgs>()),
                ("extract", public_schema::<LightpandaExtractArgs>()),
                ("tree", public_schema::<LightpandaTreeArgs>()),
                ("nodeDetails", public_schema::<LightpandaNodeDetailsArgs>()),
                (
                    "interactiveElements",
                    public_schema::<LightpandaInteractiveElementsArgs>(),
                ),
                (
                    "structuredData",
                    public_schema::<LightpandaStructuredDataArgs>(),
                ),
                ("detectForms", public_schema::<LightpandaDetectFormsArgs>()),
                ("click", public_schema::<LightpandaClickArgs>()),
                ("fill", public_schema::<LightpandaFillArgs>()),
                ("scroll", public_schema::<LightpandaScrollArgs>()),
                (
                    "waitForSelector",
                    public_schema::<LightpandaWaitForSelectorArgs>(),
                ),
                (
                    "waitForScript",
                    public_schema::<LightpandaWaitForScriptArgs>(),
                ),
                (
                    "waitForState",
                    public_schema::<LightpandaWaitForStateArgs>(),
                ),
                ("hover", public_schema::<LightpandaHoverArgs>()),
                ("press", public_schema::<LightpandaPressArgs>()),
                (
                    "selectOption",
                    public_schema::<LightpandaSelectOptionArgs>(),
                ),
                ("setChecked", public_schema::<LightpandaSetCheckedArgs>()),
                ("findElement", public_schema::<LightpandaFindElementArgs>()),
                ("consoleLogs", public_schema::<LightpandaConsoleLogsArgs>()),
                ("getUrl", public_schema::<LightpandaGetUrlArgs>()),
                ("getCookies", public_schema::<LightpandaGetCookiesArgs>()),
                ("getEnv", public_schema::<LightpandaGetEnvArgs>()),
                ("save", public_schema::<LightpandaSaveArgs>()),
                ("session_new", public_schema::<LightpandaSessionNewArgs>()),
                ("session_list", public_schema::<LightpandaSessionListArgs>()),
                (
                    "session_close",
                    public_schema::<LightpandaSessionCloseArgs>(),
                ),
            ]);
        super::super::contract::NATIVE_TOOL_MAPPINGS
            .iter()
            .map(|(native, _public)| {
                let schema = schemas
                    .remove(*native)
                    .unwrap_or_else(|| panic!("missing public argument type for {native}"));
                (*native, schema)
            })
            .collect()
    }

    /// Resolves one local `$ref` against the schema root.
    fn resolve_ref(schema: &Value, root: &Value) -> Value {
        if let Some(ref_path) = schema.get("$ref").and_then(Value::as_str) {
            if let Some(pointer) = ref_path.strip_prefix('#') {
                if let Some(resolved) = root.pointer(pointer) {
                    return resolved.clone();
                }
            }
        }
        schema.clone()
    }

    /// Reduces a property schema to its effective non-null branch, resolving
    /// `$ref`s and `anyOf: [X, null]` unions so schemars' nullable encoding
    /// compares against upstream's plain types.
    fn effective(schema: &Value, root: &Value) -> Value {
        let mut current = resolve_ref(schema, root);
        if let Some(branches) = current.get("anyOf").and_then(Value::as_array).cloned() {
            let non_null: Vec<Value> = branches
                .into_iter()
                .filter(|branch| branch.get("type").and_then(Value::as_str) != Some("null"))
                .map(|branch| resolve_ref(&branch, root))
                .collect();
            if non_null.len() == 1 {
                current = non_null.into_iter().next().unwrap();
            }
        }
        current
    }

    fn base_type(schema: &Value) -> Option<String> {
        match schema.get("type") {
            Some(Value::String(t)) => Some(t.clone()),
            Some(Value::Array(types)) => types
                .iter()
                .filter_map(Value::as_str)
                .find(|t| *t != "null")
                .map(str::to_string),
            _ => None,
        }
    }

    fn enum_values(schema: &Value) -> Option<BTreeSet<String>> {
        schema.get("enum").and_then(Value::as_array).map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
    }

    fn required_set(schema: &Value) -> BTreeSet<String> {
        schema
            .get("required")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn assert_parity(tool: &str, public: &Value, pinned: &Value) {
        let empty = serde_json::Map::new();
        let public_props = public
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let pinned_props = pinned
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let public_names: BTreeSet<&String> = public_props.keys().collect();
        let pinned_names: BTreeSet<&String> = pinned_props.keys().collect();
        assert_eq!(public_names, pinned_names, "{tool}: property names differ");
        for name in &pinned_names {
            let public_prop = effective(&public_props[*name], public);
            let pinned_prop = effective(&pinned_props[*name], pinned);
            assert_eq!(
                base_type(&public_prop),
                base_type(&pinned_prop),
                "{tool}.{name}: type differs"
            );
            assert_eq!(
                enum_values(&public_prop),
                enum_values(&pinned_prop),
                "{tool}.{name}: enum values differ"
            );
            assert_eq!(
                public_prop.get("default"),
                pinned_prop.get("default"),
                "{tool}.{name}: default differs"
            );
        }
        assert_eq!(
            required_set(public),
            required_set(pinned),
            "{tool}: required set differs"
        );
    }

    #[test]
    fn public_schemas_match_pinned_native_contract() {
        let pinned: std::collections::HashMap<String, Value> =
            super::super::contract::pinned_contract()
                .unwrap()
                .tools
                .into_iter()
                .map(|tool| (tool.name, tool.input_schema))
                .collect();
        let mut checked = 0usize;
        for (tool, mut public) in native_public_arg_schemas() {
            let pinned_schema = pinned
                .get(tool)
                .unwrap_or_else(|| panic!("pinned contract has no schema for {tool}"));
            // session_id is Local Workspaces routing metadata, never part of
            // the native contract.
            if let Some(props) = public.get_mut("properties").and_then(Value::as_object_mut) {
                props.remove("session_id");
            }
            assert_parity(tool, &public, pinned_schema);
            checked += 1;
        }
        assert_eq!(checked, super::super::contract::EXPECTED_NATIVE_TOOL_COUNT);
    }

    #[test]
    fn upstream_contract_does_not_own_session_id() {
        for tool in super::super::contract::pinned_contract().unwrap().tools {
            assert!(
                tool.input_schema
                    .pointer("/properties/session_id")
                    .is_none(),
                "upstream tool {} now owns reserved session_id",
                tool.name
            );
        }
    }

    #[test]
    fn session_id_is_present_exactly_where_allowed() {
        for (tool, schema) in native_public_arg_schemas() {
            let is_control = matches!(tool, "session_new" | "session_list" | "session_close");
            let has = schema.pointer("/properties/session_id").is_some();
            assert_eq!(has, !is_control, "{tool}: session_id placement wrong");
            assert!(
                !required_set(&schema).contains("session_id"),
                "{tool}: session_id must stay optional"
            );
        }
    }

    #[test]
    fn split_session_route_strips_only_session_id() {
        let args = LightpandaGotoArgs {
            url: "https://example.com/".into(),
            timeout: Some(5000),
            wait_until: Some(LightpandaLoadState::NetworkIdle),
            session_id: Some("alpha".into()),
        };
        let (session_id, native) = split_session_route(&args).unwrap();
        assert_eq!(session_id.as_deref(), Some("alpha"));
        assert_eq!(native["url"], "https://example.com/");
        assert_eq!(native["timeout"], 5000);
        assert_eq!(native["waitUntil"], "networkidle");
        assert!(!native.contains_key("session_id"));

        let omitted = LightpandaGetUrlArgs { session_id: None };
        let (session_id, native) = split_session_route(&omitted).unwrap();
        assert_eq!(session_id, None);
        assert!(native.is_empty());
    }

    #[test]
    fn native_navigation_policy_gates_only_navigating_urls() {
        use serde_json::json;
        // Navigating tools are gated.
        let err = validate_native_navigation(
            "goto",
            json!({"url": "file:///etc/passwd"}).as_object().unwrap(),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        assert!(validate_native_navigation(
            "goto",
            json!({"url": "https://example.com/"}).as_object().unwrap()
        )
        .is_ok());
        // Optional url absent: upstream schema requirement owns it.
        assert!(validate_native_navigation("markdown", json!({}).as_object().unwrap()).is_ok());
        // getCookies.url scopes output, never navigates.
        assert!(validate_native_navigation(
            "getCookies",
            json!({"url": "file:///etc/passwd"}).as_object().unwrap()
        )
        .is_ok());
        // Non-URL tools pass untouched.
        assert!(validate_native_navigation(
            "click",
            json!({"selector": "#x"}).as_object().unwrap()
        )
        .is_ok());
    }

    #[test]
    fn audit_detail_keeps_metadata_and_drops_raw_content() {
        use serde_json::json;
        let args = json!({
            "script": "SECRET_SCRIPT_BODY",
            "url": "https://example.com/private/path?token=abc",
            "save": "job.js"
        });
        let detail =
            safe_native_audit_detail("evaluate", args.as_object().unwrap(), Some("my-session"));
        let serialized = serde_json::to_string(&detail).unwrap();
        assert!(!serialized.contains("SECRET_SCRIPT_BODY"));
        assert!(!serialized.contains("my-session"));
        assert!(!serialized.contains("private/path"));
        assert!(!serialized.contains("token=abc"));
        assert_eq!(detail["script_bytes"], 18);
        assert_eq!(detail["origin"], "https://example.com");
        assert_eq!(detail["save_present"], true);
        assert_eq!(detail["session_present"], true);
        assert_eq!(detail["session_bytes"], 10);

        let fill = json!({"backendNodeId": 7, "value": "SECRET_VALUE"});
        let detail = safe_native_audit_detail("fill", fill.as_object().unwrap(), None);
        let serialized = serde_json::to_string(&detail).unwrap();
        assert!(!serialized.contains("SECRET_VALUE"));
        assert_eq!(detail["target"], "backend_node_id");
        assert_eq!(detail["value_bytes"], 12);
        assert_eq!(detail["session_present"], false);

        let close = json!({"id": "secret-session-name"});
        let detail = safe_native_audit_detail("session_close", close.as_object().unwrap(), None);
        let serialized = serde_json::to_string(&detail).unwrap();
        assert!(!serialized.contains("secret-session-name"));
        assert_eq!(detail["id_bytes"], 19);
    }
}

#[cfg(test)]
mod ref_adapter_tests {
    use super::*;

    fn collect_ids(text: &str) -> (String, Vec<u64>) {
        let mut ids = Vec::new();
        let out = normalize_tree_text(text, &mut |id| {
            ids.push(id);
            format!("e{}", ids.len())
        });
        (out, ids)
    }

    #[test]
    fn is_valid_ref_accepts_only_e_plus_digits() {
        assert!(is_valid_ref("e1"));
        assert!(is_valid_ref("e42"));
        assert!(!is_valid_ref("E1"));
        assert!(!is_valid_ref("e"));
        assert!(!is_valid_ref("e1x"));
        assert!(!is_valid_ref("1"));
        assert!(!is_valid_ref(""));
        assert!(!is_valid_ref(" e1"));
    }

    #[test]
    fn parse_node_selector_requires_non_blank_selector() {
        let selector =
            parse_node_selector(r##"{"backendNodeId":4,"tagName":"button","selector":"#submit"}"##);
        assert_eq!(selector.as_deref(), Some("#submit"));
        assert_eq!(parse_node_selector(r#"{"tagName":"button"}"#), None);
        assert_eq!(parse_node_selector(r#"{"selector":"  "}"#), None);
        assert_eq!(parse_node_selector("not json"), None);
    }

    #[test]
    fn normalize_tree_text_replaces_only_leading_ids() {
        let input = "1 WebArea 'App'\n 2 [i] textbox 'Name' value='42'\nno id line\n";
        let (out, ids) = collect_ids(input);
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(
            out,
            "e1 WebArea 'App'\n e2 [i] textbox 'Name' value='42'\nno id line\n"
        );
    }

    #[test]
    fn normalize_tree_text_handles_repeated_ids_and_no_trailing_newline() {
        let input = "7 StaticText 'x'\n 7 StaticText 'x'";
        let mut ids = Vec::new();
        let out = normalize_tree_text(input, &mut |id| {
            if let Some(pos) = ids.iter().position(|seen| *seen == id) {
                return format!("e{}", pos + 1);
            }
            ids.push(id);
            format!("e{}", ids.len())
        });
        assert_eq!(ids, vec![7]);
        assert_eq!(out, "e1 StaticText 'x'\n e1 StaticText 'x'");
    }

    #[test]
    fn normalize_tree_text_empty_input_stays_empty() {
        let (out, ids) = collect_ids("");
        assert_eq!(out, "");
        assert!(ids.is_empty());
    }

    #[test]
    fn normalize_backend_node_ids_renames_fields_and_recurses() {
        let doc = serde_json::json!({
            "items": [
                {"backendNodeId": 3, "text": "Docs", "href": "https://example.test/docs"},
                {"backendNodeId": "bogus", "text": "Bad"}
            ],
            "backendNodeId": 1,
            "nested": {"backendNodeId": 2}
        });
        let mut ids = Vec::new();
        let out = normalize_backend_node_ids(doc, &mut |id| {
            ids.push(id);
            format!("e{}", ids.len())
        });
        // serde_json maps iterate in sorted key order, so the top-level
        // backendNodeId registers before the nested ones.
        assert_eq!(out["ref"], "e1");
        assert_eq!(out["items"][0]["ref"], "e2");
        assert_eq!(out["items"][0].get("backendNodeId"), None);
        // Non-integer ids are dropped, never passed through.
        assert_eq!(out["items"][1].get("backendNodeId"), None);
        assert_eq!(out["items"][1].get("ref"), None);
        assert_eq!(out["nested"]["ref"], "e3");
        assert_eq!(ids, vec![1, 3, 2]);
    }

    #[test]
    fn redact_form_passwords_forces_password_values_to_stars() {
        let mut doc = serde_json::json!([
            {
                "backendNodeId": 2,
                "action": "/login",
                "fields": [
                    {"backendNodeId": 3, "inputType": "text", "value": "visible-user"},
                    {"backendNodeId": 4, "inputType": "password", "value": "hunter2-secret"},
                    {"backendNodeId": 5, "inputType": "PASSWORD", "value": "loud-secret"},
                    {"backendNodeId": 6, "inputType": "password"}
                ]
            }
        ]);
        redact_form_passwords(&mut doc);
        let fields = &doc[0]["fields"];
        assert_eq!(
            fields[0]["value"], "visible-user",
            "non-password value stays"
        );
        assert_eq!(fields[1]["value"], "*****");
        assert_eq!(fields[2]["value"], "*****", "match is case-insensitive");
        assert_eq!(fields[3].get("value"), None, "absent value is not invented");
        let serialized = serde_json::to_string(&doc).unwrap();
        assert!(!serialized.contains("hunter2-secret"));
        assert!(!serialized.contains("loud-secret"));
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    #[test]
    fn content_cap_is_bounded() {
        assert_eq!(validated_content_cap(None).unwrap(), MAX_CONTENT_BYTES);
        assert_eq!(validated_content_cap(Some(1)).unwrap(), 1);
        assert_eq!(
            validated_content_cap(Some(MAX_CONTENT_BYTES as u64)).unwrap(),
            MAX_CONTENT_BYTES
        );
        for bad in [0u64, MAX_CONTENT_BYTES as u64 + 1, u64::MAX] {
            let err = validated_content_cap(Some(bad)).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument);
        }
    }

    #[test]
    fn find_element_requires_one_bounded_criterion() {
        assert!(validate_find_element(None, None).is_err());
        assert!(validate_find_element(Some("  "), Some("")).is_err());
        assert!(validate_find_element(Some("button"), None).is_ok());
        assert!(validate_find_element(None, Some("Submit")).is_ok());
        assert!(validate_find_element(Some(&"r".repeat(129)), None).is_err());
        assert!(validate_find_element(None, Some(&"n".repeat(513))).is_err());
    }

    #[test]
    fn scroll_offsets_are_bounded() {
        assert!(validate_scroll_offsets(None, None).is_ok());
        assert!(validate_scroll_offsets(Some(0), Some(-100)).is_ok());
        assert!(validate_scroll_offsets(Some(MAX_SCROLL_OFFSET), Some(-MAX_SCROLL_OFFSET)).is_ok());
        assert!(validate_scroll_offsets(Some(MAX_SCROLL_OFFSET + 1), None).is_err());
        assert!(validate_scroll_offsets(None, Some(-(MAX_SCROLL_OFFSET + 1))).is_err());
    }

    #[test]
    fn extract_schema_requires_object_root() {
        for bad in [
            serde_json::json!(["title"]),
            serde_json::json!("title"),
            serde_json::json!(null),
        ] {
            let err = validate_extract_schema(&bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument);
        }
    }

    #[test]
    fn extract_schema_accepts_valid_shapes_and_encodes_once() {
        let schema = serde_json::json!({
            "title": "h1",
            "items": [{"selector": ".row", "fields": {"name": ".n"}, "limit": 3}],
            "link": {"selector": "a.more", "attr": "href"}
        });
        let encoded = validate_extract_schema(&schema).unwrap();
        assert_eq!(encoded, serde_json::to_string(&schema).unwrap());
    }

    #[test]
    fn extract_schema_rejects_excessive_limits() {
        for bad in [
            serde_json::json!({"items": [{"selector": ".row", "limit": -1}]}),
            serde_json::json!({"items": [{"selector": ".row", "limit": 10_001}]}),
            serde_json::json!({"items": [{"selector": ".row", "limit": "5"}]}),
        ] {
            let err = validate_extract_schema(&bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument);
        }
        assert!(validate_extract_schema(
            &serde_json::json!({"items": [{"selector": ".row", "limit": 10_000}]})
        )
        .is_ok());
    }

    #[test]
    fn extract_schema_rejects_deep_nesting_and_too_many_selectors() {
        // 9 levels of nesting: one past the bound.
        let mut deep = serde_json::json!({"sel": ".x"});
        for _ in 0..MAX_EXTRACT_SCHEMA_DEPTH {
            deep = serde_json::json!({"fields": deep});
        }
        assert!(validate_extract_schema(&deep).is_err());

        let many = serde_json::Value::Array(
            (0..=MAX_EXTRACT_SCHEMA_STRINGS)
                .map(|i| serde_json::json!(format!(".s{i}")))
                .collect(),
        );
        let many = serde_json::json!({"items": many});
        assert!(validate_extract_schema(&many).is_err());
    }

    #[test]
    fn extract_schema_rejects_oversized_documents() {
        let big = serde_json::json!({"field": "x".repeat(MAX_EXTRACT_SCHEMA_BYTES)});
        let err = validate_extract_schema(&big).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn encode_js_string_is_safe_json_literal() {
        assert_eq!(encode_js_string("abc"), "\"abc\"");
        assert_eq!(encode_js_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
        // Newlines and quotes can never break out of the literal.
        assert_eq!(encode_js_string("'); alert(1); ('"), "\"'); alert(1); ('\"");
    }

    #[test]
    fn page_evaluate_script_invokes_function_literals_once() {
        assert_eq!(
            page_evaluate_script("() => document.title", None),
            "(() => document.title)()"
        );
        assert_eq!(
            page_evaluate_script("async () => 1", None),
            "(async () => 1)()"
        );
        assert_eq!(
            page_evaluate_script("function() { return 1; }", None),
            "(function() { return 1; })()"
        );
        // Bare expressions pass through untouched.
        assert_eq!(
            page_evaluate_script("document.title", None),
            "document.title"
        );
    }

    #[test]
    fn page_evaluate_script_binds_element_target() {
        assert_eq!(
            page_evaluate_script("(el) => el.textContent", Some("#nd-4")),
            "((el) => el.textContent)(document.querySelector(\"#nd-4\"))"
        );
        assert_eq!(
            page_evaluate_script("el.tagName", Some("#nd-4")),
            "((el) => (el.tagName))(document.querySelector(\"#nd-4\"))"
        );
    }

    #[test]
    fn fixed_lightpanda_scripts_have_no_interpolation_points() {
        for script in [
            PERFORMANCE_TIMING_SCRIPT,
            NETWORK_RESOURCE_TIMING_SCRIPT,
            DOM_OUTER_SCRIPT,
            DRAG_SYNTHETIC_SCRIPT,
            HOVER_SYNTHETIC_SCRIPT,
        ] {
            assert!(!script.contains("{0}"), "fixed script has a format slot");
            assert!(!script.contains("${"), "fixed script has a template slot");
        }
    }

    #[test]
    fn dom_outer_script_sanitizes_outerhtml_from_a_clone() {
        // Sanitization must happen on a clone, not the live element.
        assert!(
            DOM_OUTER_SCRIPT.contains("cloneNode(true)"),
            "must clone before serializing"
        );
        assert!(
            DOM_OUTER_SCRIPT.contains("[redacted]"),
            "must redact sensitive values"
        );
        assert!(
            DOM_OUTER_SCRIPT.contains("setAttribute"),
            "must overwrite sensitive attributes"
        );
        // The serialized markup must come from the clone, never the live element.
        assert!(
            DOM_OUTER_SCRIPT.contains("clone.outerHTML"),
            "must serialize the clone"
        );
        assert!(
            !DOM_OUTER_SCRIPT.contains("element.outerHTML"),
            "must not serialize the live element's outerHTML directly"
        );
        // Lightpanda performs no layout; a rect would be fabricated.
        assert!(
            !DOM_OUTER_SCRIPT.contains("getBoundingClientRect"),
            "must not read a layout rect"
        );
    }

    #[test]
    fn drag_synthetic_script_binds_selectors_as_data() {
        let script = drag_synthetic_script("#nd-1", "#nd-2");
        assert!(
            script.ends_with(")(\"#nd-1\",\"#nd-2\")"),
            "selectors must be bound as JSON string literals: {script}"
        );
        let hostile = drag_synthetic_script("\"); evil(); (", "#nd-2");
        assert!(
            hostile.contains("\"\\\"); evil(); (\""),
            "hostile selector must stay inside the string literal: {hostile}"
        );
        assert!(
            !hostile.contains("); evil(); (,"),
            "hostile selector escaped the literal: {hostile}"
        );
    }

    #[test]
    fn hover_synthetic_script_binds_selector_as_data() {
        let script = hover_synthetic_script("#nd-4");
        assert!(
            script.ends_with(")(\"#nd-4\")"),
            "selector must be bound as a JSON string literal: {script}"
        );
        let hostile = hover_synthetic_script("\"); evil(); (");
        assert!(
            hostile.contains("\"\\\"); evil(); (\""),
            "hostile selector must stay inside the string literal: {hostile}"
        );
    }

    #[test]
    fn is_static_initiator_classifies_subresource_types() {
        for initiator in [
            "img", "image", "script", "link", "css", "font", "iframe", "media",
        ] {
            assert!(is_static_initiator(initiator), "{initiator}");
        }
        for initiator in ["fetch", "xmlhttprequest", "beacon", "other", ""] {
            assert!(!is_static_initiator(initiator), "{initiator}");
        }
    }

    #[test]
    fn redact_sensitive_url_masks_credential_like_query_values() {
        assert_eq!(
            redact_sensitive_url("https://ex.test/a?token=abc&page=2"),
            "https://ex.test/a?token=[redacted]&page=2"
        );
        assert_eq!(
            redact_sensitive_url("https://ex.test/a?API_KEY=x&Session=y&q=z"),
            "https://ex.test/a?API_KEY=[redacted]&Session=[redacted]&q=z"
        );
        // No query string: untouched.
        assert_eq!(
            redact_sensitive_url("https://ex.test/a"),
            "https://ex.test/a"
        );
        // A sensitive key without a value is not a leak.
        assert_eq!(
            redact_sensitive_url("https://ex.test/a?token"),
            "https://ex.test/a?token"
        );
    }
}
