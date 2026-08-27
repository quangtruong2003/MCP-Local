//! MCP tool server: 7 workspace-scoped file tools behind the fail-closed
//! audit pipeline.
//!
//! Note: requests rmcp rejects before reaching a handler (malformed args,
//! unknown tool names) produce a protocol error with no audit row; the
//! one-audit-row-per-request guarantee covers requests that reach these
//! handlers. Flagged for spec Task 15 ruling.

use crate::{
    browser::{
        files::{BrowserDownloadInfo, BrowserSavedDownloadOutput},
        mcp::{
            call_result_text, parse_tabs_markdown, truncate_utf8, BrowserConsoleArgs,
            BrowserConsoleOutput, BrowserDevtoolsOutput, BrowserDialogArgs, BrowserDomArgs,
            BrowserDomOutput, BrowserDragArgs, BrowserEmulateArgs, BrowserEmulationRequest,
            BrowserEvaluateArgs, BrowserEvaluateOutput, BrowserFillArgs, BrowserLoadState,
            BrowserNavigateArgs, BrowserNavigationAction, BrowserNavigationOutput,
            BrowserNetworkArgs, BrowserNetworkOutput, BrowserOkOutput, BrowserOpenArgs,
            BrowserOpenOutput, BrowserOrientation, BrowserPageSummary, BrowserPerformanceOutput,
            BrowserPressArgs, BrowserRunStepsArgs, BrowserRunStepsOutput, BrowserSaveDownloadArgs,
            BrowserScreenshotArgs, BrowserScreenshotMetadata, BrowserScreenshotMode,
            BrowserSearchArgs, BrowserSelectArgs, BrowserSnapshotOutput, BrowserStep,
            BrowserStepError, BrowserTabAction, BrowserTabsArgs, BrowserTargetArgs,
            BrowserTextOutput, BrowserTextTruncatedOutput, BrowserTraceAction, BrowserTraceArgs,
            BrowserTraceOutput, BrowserUploadArgs, BrowserUploadOutput, BrowserUploadSource,
            BrowserViewportArgs, BrowserViewportOutput, BrowserWaitArgs, BrowserWaitCondition,
        },
        playwright::{
            DOM_INSPECT_FUNCTION, PERFORMANCE_FUNCTION, WAIT_READY_FUNCTION, WAIT_URL_FUNCTION,
            WAIT_VISIBLE_FUNCTION,
        },
        policy::{
            bounded_event_limit, redacted_origin, validate_batch_step_count,
            validate_navigation_url, validate_timeout, validate_viewport,
        },
        types::{
            BrowserSessionStatus, DEFAULT_SCREENSHOT_MAX_EDGE, MAX_SCREENSHOT_ENCODED_BYTES,
            MAX_SCREENSHOT_PIXELS,
        },
    },
    screen_capture::{
        filter_targets_for_scope, CaptureTarget, CapturedImage, ScreenCaptureProvider,
        SystemScreenCaptureProvider,
    },
};
use axum::Router;
use base64::Engine as _;
use image::{imageops::FilterType, GenericImageView, ImageFormat};
use lc_core::error::ErrorCode;
use lc_core::pipeline::{Audited, RequiredCapability};
use lc_core::{error::CoreError, CoreServices};
use rmcp::{
    handler::server::{
        common::schema_for_output, router::prompt::PromptRouter, wrapper::Parameters,
    },
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo},
    prompt_handler, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ServerHandler,
};
use std::{
    io::Cursor,
    sync::Arc,
    time::{Duration, Instant},
};

/// Builds a success result carrying both the pretty-printed text (for
/// clients that ignore schemas) and the typed `structuredContent` payload
/// (for clients that validate against the tool's outputSchema).
fn ok_structured<T: serde::Serialize>(v: &T) -> CallToolResult {
    let json = serde_json::to_value(v).unwrap_or(serde_json::Value::Null);
    let mut r = ok_json(&json);
    r.structured_content = Some(json);
    r
}

/// Serializes a CoreError as an `isError` tool result carrying the stable
/// error code.
fn err_result(e: &CoreError) -> CallToolResult {
    let payload = serde_json::json!({
        "code": e.code.as_str(),
        "message": e.message,
        "detail": e.detail,
    });
    CallToolResult::error(vec![ContentBlock::text(payload.to_string())])
}

fn ok_json<T: serde::Serialize>(v: &T) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(v).unwrap_or_default(),
    )])
}

#[derive(Clone)]
pub struct WorkspaceServer {
    core: Arc<CoreServices>,
    agent_policy: Arc<crate::settings::AgentPolicyState>,
    agent_settings: Arc<crate::settings::AgentSettingsController>,
    browser: Arc<crate::browser::manager::BrowserManager>,
    capture_provider: Arc<dyn ScreenCaptureProvider>,
    pub(crate) prompt_router: PromptRouter<Self>,
}

/// Client-facing workspace summary. Deliberately omits `root_path`: the
/// absolute filesystem location is manager-UI information, not something the
/// MCP client needs.
#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct WorkspaceSummary {
    pub id: String,
    pub name: String,
    pub can_read: bool,
    pub can_write: bool,
    pub can_execute: bool,
    pub execution_mode: lc_core::models::ExecutionMode,
}

fn summary_of(ws: &lc_core::models::Workspace) -> WorkspaceSummary {
    WorkspaceSummary {
        id: ws.id.clone(),
        name: ws.name.clone(),
        can_read: ws.can_read,
        can_write: ws.can_write,
        can_execute: ws.can_execute,
        execution_mode: ws.execution_mode,
    }
}

#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct AgentPolicyOutput {
    pub mode: crate::settings::AgentMode,
    pub paused: bool,
    pub unrestricted_access: bool,
    pub screen_capture_enabled: bool,
    pub capture_scope: crate::settings::CaptureScope,
    pub visual_verification: bool,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct EmptyArgs {}

#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct UpdateAgentSettingsOutput {
    pub mode: crate::settings::AgentMode,
    pub screen_capture_enabled: bool,
    pub capture_scope: crate::settings::CaptureScope,
    pub visual_verification: bool,
    pub changed_fields: Vec<String>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct CaptureScreenArgs {
    pub target_id: String,
    #[serde(default)]
    pub max_edge: Option<u32>,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct CaptureMetadataOutput {
    pub target_id: String,
    pub target_kind: crate::screen_capture::CaptureTargetKind,
    pub name: String,
    pub application: Option<String>,
    pub original_width: u32,
    pub original_height: u32,
    pub returned_width: u32,
    pub returned_height: u32,
    pub encoded_bytes: usize,
    pub sha256: String,
}

// ---- argument structs (all take workspace_id first) ----

fn default_dot() -> String {
    ".".into()
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ListDirArgs {
    pub workspace_id: String,
    /// Path relative to the workspace root, e.g. "." or "src"
    #[serde(default = "default_dot")]
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct FindArgs {
    pub workspace_id: String,
    /// Glob pattern relative to the workspace root, e.g. "**/*.rs"
    pub pattern: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    pub workspace_id: String,
    pub query: String,
    /// Restrict search to this subtree
    #[serde(default)]
    pub path_prefix: Option<String>,
    #[serde(default)]
    pub case_sensitive: bool,
    /// Treat query as a regular expression (default: literal text)
    #[serde(default)]
    pub is_regex: bool,
    #[serde(default)]
    pub max_results: Option<usize>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ReadArgs {
    pub workspace_id: String,
    pub path: String,
    #[serde(default)]
    pub offset_line: Option<u64>,
    #[serde(default)]
    pub max_lines: Option<u64>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ReadManyArgs {
    pub workspace_id: String,
    /// Up to 8 paths relative to the workspace root
    pub paths: Vec<String>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct CreateArgs {
    pub workspace_id: String,
    pub path: String,
    pub content: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct PatchArgs {
    pub workspace_id: String,
    pub path: String,
    /// Exact text to replace; must occur exactly once in the current file
    pub old_string: String,
    pub new_string: String,
    /// sha256 revision from the last read_file of this path
    pub base_revision: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct RunCommandArgs {
    pub workspace_id: String,
    /// Approved command basename, e.g. "git" — never a path
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory relative to the workspace root (default ".")
    #[serde(default)]
    pub cwd: Option<String>,
    /// Kill the process tree after this many milliseconds (max 600000)
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct RunPowerShellArgs {
    pub workspace_id: String,
    /// PowerShell script executed via stdin (requires FullPowerShell mode)
    pub script: String,
    /// Working directory relative to the workspace root (default ".")
    #[serde(default)]
    pub cwd: Option<String>,
    /// Kill the process tree after this many milliseconds (max 600000)
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Client-facing mirror of the runner's `ExecutionResult` (which stays
/// policy-internal and unserialized). ChatGPT gets partial output on timeout.
#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct ExecutionOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub duration_ms: u64,
}

/// Maps a runner/policy failure to a stable CoreError. PolicyError codes that
/// are already stable pass through as-is (`COMMAND_NOT_ALLOWED`,
/// `INVALID_ARGUMENT`, and Core-inherited codes like
/// `PATH_OUTSIDE_WORKSPACE`); anything else is an execution failure.
fn execution_error(e: crate::execution::policy::PolicyError) -> CoreError {
    match e.code {
        "COMMAND_NOT_ALLOWED" => CoreError::new(ErrorCode::CommandNotAllowed, e.message),
        "INVALID_ARGUMENT" => CoreError::new(ErrorCode::InvalidArgument, e.message),
        "PATH_OUTSIDE_WORKSPACE" => CoreError::new(ErrorCode::PathOutsideWorkspace, e.message),
        _ => CoreError::new(ErrorCode::ExecutionFailed, e.message),
    }
}

async fn pages_from_tab_result(
    browser: &crate::browser::manager::BrowserManager,
    result: &CallToolResult,
) -> Vec<BrowserPageSummary> {
    let parsed = parse_tabs_markdown(&call_result_text(result));
    browser
        .reconcile_pages(parsed)
        .await
        .into_iter()
        .map(|page| BrowserPageSummary {
            page_id: page.page_id,
            title: page.title,
            url: page.url,
            active: page.active,
        })
        .collect()
}

async fn refresh_browser_tabs(
    browser: &crate::browser::manager::BrowserManager,
) -> Result<Vec<BrowserPageSummary>, CoreError> {
    let mut args = serde_json::Map::new();
    args.insert("action".into(), serde_json::Value::String("list".into()));
    let result = browser.call_upstream("browser_tabs", args).await?;
    Ok(pages_from_tab_result(browser, &result).await)
}

fn browser_outcome<T: serde::Serialize>(
    core: &CoreServices,
    op: &str,
    result: Result<T, CoreError>,
    detail: Option<serde_json::Value>,
) -> CallToolResult {
    let error = result.as_ref().err();
    if let Err(audit_error) = crate::browser::audit::record_browser_call(core, op, error, detail) {
        return err_result(&audit_error);
    }
    match result {
        Ok(value) => ok_structured(&value),
        Err(error) => err_result(&error),
    }
}

/// Opaque pagination token for console/network reads. Encodes the number of
/// lines already consumed; callers must treat it as opaque.
fn encode_cursor(offset: usize) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("lc1:{offset}"))
}

fn decode_cursor(cursor: &str) -> Result<usize, CoreError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cursor)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| {
            text.strip_prefix("lc1:")
                .and_then(|offset| offset.parse::<usize>().ok())
        });
    bytes.ok_or_else(|| {
        CoreError::new(
            ErrorCode::InvalidArgument,
            "browser cursor must come from a previous console/network call",
        )
    })
}

/// Takes one page of lines starting at the decoded cursor offset. Returns the
/// page text and the next cursor (None when no lines remain).
fn paginate_lines(
    text: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<(String, Option<String>), CoreError> {
    let offset = match cursor {
        Some(cursor) => decode_cursor(cursor)?,
        None => 0,
    };
    let lines: Vec<&str> = text.lines().skip(offset).take(limit).collect();
    let page = lines.join("\n");
    let consumed = offset + lines.len();
    let next = if text.lines().nth(consumed).is_some() {
        Some(encode_cursor(consumed))
    } else {
        None
    };
    Ok((page, next))
}

/// Histogram of console message levels in a returned page. Audit stores only
/// these counts, never message text.
fn console_level_histogram(text: &str) -> std::collections::BTreeMap<String, u64> {
    let mut levels = std::collections::BTreeMap::<String, u64>::new();
    for line in text.lines() {
        let level = console_line_level(line);
        *levels.entry(level).or_default() += 1;
    }
    levels
}

/// Level of one console line. Pinned upstream 0.0.79 renders messages as
/// `[ERROR] boom @ https://example.test/app.js:12`; the simpler
/// `error: boom` shape is accepted as a fallback. Anything else is "other".
fn console_line_level(line: &str) -> String {
    let trimmed = line.trim();
    let prefix = if let Some(rest) = trimmed.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        trimmed.split(':').next().unwrap_or_default()
    };
    match prefix.trim().to_ascii_lowercase().as_str() {
        level @ ("error" | "warning" | "info" | "debug") => level.to_string(),
        _ => "other".to_string(),
    }
}

/// Strips the pinned upstream 0.0.79 presentation header from console output:
/// `Total messages: N (Errors: E, Warnings: W)`, the optional
/// `Returning M messages for level "X"` line, and the separating blank line.
/// Message text itself is never altered.
fn strip_console_header(text: &str) -> &str {
    let mut offset = 0;
    let mut saw_header = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("Total messages:")
            || (saw_header && trimmed.starts_with("Returning "))
        {
            saw_header = true;
            offset += line.len();
            continue;
        }
        if saw_header && trimmed.is_empty() {
            offset += line.len();
            continue;
        }
        break;
    }
    &text[offset..]
}

fn redact_network_text(text: String) -> String {
    const DENY: &[&str] = &[
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "x-api-key",
    ];
    text.lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if DENY
                .iter()
                .any(|name| lower.trim_start().starts_with(&format!("{name}:")))
            {
                let name = line.split(':').next().unwrap_or("header");
                format!("{name}: [redacted]")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn browser_wait_internal(
    browser: &crate::browser::manager::BrowserManager,
    condition: BrowserWaitCondition,
    requested_timeout: Option<u64>,
) -> Result<BrowserOkOutput, CoreError> {
    let timeout_ms = validate_timeout(requested_timeout, false)?;
    match condition {
        BrowserWaitCondition::TextPresent { text } => {
            let mut args = serde_json::Map::new();
            args.insert("text".into(), serde_json::Value::String(text));
            browser.call_upstream("browser_wait_for", args).await?;
        }
        BrowserWaitCondition::TextAbsent { text } => {
            let mut args = serde_json::Map::new();
            args.insert("textGone".into(), serde_json::Value::String(text));
            browser.call_upstream("browser_wait_for", args).await?;
        }
        BrowserWaitCondition::Delay { milliseconds } => {
            if milliseconds == 0 || milliseconds > timeout_ms {
                return Err(CoreError::new(
                    ErrorCode::InvalidArgument,
                    "browser wait delay must be within timeout",
                ));
            }
            let mut args = serde_json::Map::new();
            args.insert(
                "time".into(),
                serde_json::json!(milliseconds as f64 / 1000.0),
            );
            browser.call_upstream("browser_wait_for", args).await?;
        }
        other => {
            let started = Instant::now();
            loop {
                browser.guard_page_operation().await?;
                let (function, target, expected) = match &other {
                    BrowserWaitCondition::RefVisible { ref_id } => {
                        (WAIT_VISIBLE_FUNCTION, Some(ref_id.as_str()), Some(true))
                    }
                    BrowserWaitCondition::RefHidden { ref_id } => {
                        (WAIT_VISIBLE_FUNCTION, Some(ref_id.as_str()), Some(false))
                    }
                    BrowserWaitCondition::UrlContains { .. } => (WAIT_URL_FUNCTION, None, None),
                    BrowserWaitCondition::LoadState { .. } => (WAIT_READY_FUNCTION, None, None),
                    _ => unreachable!(),
                };
                let mut args = serde_json::Map::new();
                args.insert(
                    "function".into(),
                    serde_json::Value::String(function.into()),
                );
                if let Some(target) = target {
                    args.insert("target".into(), serde_json::Value::String(target.into()));
                }
                let response = browser.call_upstream("browser_evaluate", args).await?;
                let text = call_result_text(&response);
                let matched = match &other {
                    BrowserWaitCondition::RefVisible { .. }
                    | BrowserWaitCondition::RefHidden { .. } => {
                        // Upstream browser_evaluate returns JSON.stringify(result).
                        // Parse it and compare the boolean instead of substring
                        // matching, so an error message or string that merely
                        // contains "true"/"false" can never false-match.
                        serde_json::from_str::<serde_json::Value>(&text)
                            .map(|value| value == serde_json::Value::Bool(expected.unwrap()))
                            .unwrap_or(false)
                    }
                    BrowserWaitCondition::UrlContains { value } => text.contains(value),
                    BrowserWaitCondition::LoadState { state } => {
                        let wanted = match state {
                            BrowserLoadState::Loading => "loading",
                            BrowserLoadState::Interactive => "interactive",
                            BrowserLoadState::Complete => "complete",
                        };
                        text.to_ascii_lowercase().contains(wanted)
                    }
                    _ => false,
                };
                if matched {
                    break;
                }
                if started.elapsed() >= Duration::from_millis(timeout_ms) {
                    return Err(CoreError::new(
                        ErrorCode::BrowserActionTimeout,
                        "browser wait condition timed out",
                    ));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    Ok(BrowserOkOutput { ok: true })
}

/// Reads width/height from the PNG IHDR chunk so the pixel budget can be
/// enforced before any full decode allocates memory.
fn png_header_dimensions(bytes: &[u8]) -> Result<(u32, u32), CoreError> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return Err(CoreError::new(
            ErrorCode::BrowserProtocolError,
            "Playwright screenshot contained invalid PNG data",
        ));
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    Ok((width, height))
}

fn browser_screenshot_payload(
    result: &CallToolResult,
    mode: BrowserScreenshotMode,
    max_edge: Option<u32>,
) -> Result<(Vec<u8>, BrowserScreenshotMetadata), CoreError> {
    if max_edge == Some(0) {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "max_edge must be greater than zero",
        ));
    }
    let max_edge = max_edge.unwrap_or(DEFAULT_SCREENSHOT_MAX_EDGE);
    let image = result
        .content
        .iter()
        .find_map(|block| block.as_image())
        .ok_or_else(|| {
            CoreError::new(
                ErrorCode::BrowserProtocolError,
                "Playwright screenshot did not return an image",
            )
        })?;
    if image.mime_type != "image/png" {
        return Err(CoreError::new(
            ErrorCode::BrowserProtocolError,
            "Playwright screenshot returned a non-PNG image",
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&image.data)
        .map_err(|_| {
            CoreError::new(
                ErrorCode::BrowserProtocolError,
                "Playwright screenshot contained invalid base64",
            )
        })?;
    if bytes.len() > MAX_SCREENSHOT_ENCODED_BYTES {
        return Err(CoreError::new(
            ErrorCode::BrowserProtocolError,
            "Playwright screenshot exceeded the encoded byte budget",
        ));
    }
    let (header_width, header_height) = png_header_dimensions(&bytes)?;
    if u64::from(header_width) * u64::from(header_height) > MAX_SCREENSHOT_PIXELS {
        return Err(CoreError::new(
            ErrorCode::BrowserProtocolError,
            "Playwright screenshot exceeded the pixel budget",
        ));
    }
    let decoded = image::load_from_memory_with_format(&bytes, ImageFormat::Png).map_err(|_| {
        CoreError::new(
            ErrorCode::BrowserProtocolError,
            "Playwright screenshot contained invalid PNG data",
        )
    })?;
    let (ow, oh) = decoded.dimensions();
    let (rw, rh) = crate::screen_capture::scaled_dimensions(ow, oh, Some(max_edge));
    let returned = if (rw, rh) == (ow, oh) {
        decoded
    } else {
        decoded.resize_exact(rw, rh, FilterType::Lanczos3)
    };
    let mut output = Vec::new();
    returned
        .write_to(&mut Cursor::new(&mut output), ImageFormat::Png)
        .map_err(|_| {
            CoreError::new(
                ErrorCode::BrowserArtifactFailed,
                "failed to encode browser screenshot",
            )
        })?;
    let metadata = BrowserScreenshotMetadata {
        mode,
        original_width: ow,
        original_height: oh,
        returned_width: rw,
        returned_height: rh,
        encoded_bytes: output.len(),
        sha256: lc_core::ops::sha256_hex(&output),
    };
    Ok((output, metadata))
}

fn screenshot_tool_result(bytes: Vec<u8>, metadata: &BrowserScreenshotMetadata) -> CallToolResult {
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut result = CallToolResult::success(vec![
        ContentBlock::image(encoded, "image/png"),
        ContentBlock::text(serde_json::to_string_pretty(metadata).unwrap_or_default()),
    ]);
    result.structured_content = serde_json::to_value(metadata).ok();
    result
}

// The #[tool_router] macro generates an associated `Self::tool_router()`
// consumed by #[tool_handler]; no runtime field is needed.
#[tool_router]
impl WorkspaceServer {
    pub fn new(
        core: Arc<CoreServices>,
        agent_policy: Arc<crate::settings::AgentPolicyState>,
        agent_settings: Arc<crate::settings::AgentSettingsController>,
        browser: Arc<crate::browser::manager::BrowserManager>,
        capture_provider: Arc<dyn ScreenCaptureProvider>,
    ) -> Self {
        Self {
            core,
            agent_policy,
            agent_settings,
            browser,
            capture_provider,
            prompt_router: crate::agent_skills::workspace_prompt_router(),
        }
    }

    #[tool(
        description = "Read Browser Automation state without starting a browser. This remains readable when Browser Automation is disabled so the agent can report the blocker. Pause still wins.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<BrowserSessionStatus>()
    )]
    async fn browser_get_status(&self, Parameters(_): Parameters<EmptyArgs>) -> CallToolResult {
        let result = if self.core.pause.is_paused() {
            Err(CoreError::new(
                ErrorCode::AccessPaused,
                "access is paused by the user",
            ))
        } else {
            Ok(self.browser.status().await)
        };
        let detail = result.as_ref().ok().map(|status| {
            serde_json::json!({
                "enabled": status.enabled,
                "running": status.running,
                "engine": status.configured_engine,
                "run_mode": status.run_mode,
            })
        });
        browser_outcome(&self.core, "browser_get_status", result, detail)
    }

    #[tool(
        description = "Start the dedicated Local Workspaces browser session, optionally navigating to an initial http(s) URL. Uses the configured engine and Background/Visible Debug mode.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserOpenOutput>()
    )]
    async fn browser_open(&self, Parameters(a): Parameters<BrowserOpenArgs>) -> CallToolResult {
        let origin = a.url.as_deref().and_then(|url| redacted_origin(url).ok());
        let result = async {
            let url = a.url.as_deref();
            if let Some(url) = url {
                validate_navigation_url(url)?;
            }
            let status = self.browser.open(None).await?;
            let pages = refresh_browser_tabs(&self.browser).await?;
            if let Some(url) = url {
                let mut args = serde_json::Map::new();
                args.insert("url".into(), serde_json::Value::String(url.to_string()));
                self.browser.call_upstream("browser_navigate", args).await?;
            }
            Ok::<_, CoreError>(BrowserOpenOutput { status, pages })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_open",
            result,
            Some(serde_json::json!({ "origin": origin })),
        )
    }

    #[tool(
        description = "Close the active Local Workspaces browser session. The dedicated profile remains on disk for future automation sessions.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserSessionStatus>()
    )]
    async fn browser_close(&self, Parameters(_): Parameters<EmptyArgs>) -> CallToolResult {
        let result = self.browser.close().await;
        browser_outcome(&self.core, "browser_close", result, None)
    }

    #[tool(
        description = "List, create, select, or close browser tabs. Local Workspaces returns session-scoped opaque page_id values; pass page_id for select/close instead of Playwright tab indices.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<Vec<BrowserPageSummary>>()
    )]
    async fn browser_tabs(&self, Parameters(a): Parameters<BrowserTabsArgs>) -> CallToolResult {
        let action_name = match a.action {
            BrowserTabAction::List => "list",
            BrowserTabAction::New => "new",
            BrowserTabAction::Select => "select",
            BrowserTabAction::Close => "close",
        };
        let origin = a.url.as_deref().and_then(|url| redacted_origin(url).ok());
        let result = async {
            self.browser.guard_page_operation().await?;
            match a.action {
                BrowserTabAction::List => refresh_browser_tabs(&self.browser).await,
                BrowserTabAction::New => {
                    let mut args = serde_json::Map::new();
                    args.insert("action".into(), serde_json::Value::String("new".into()));
                    if let Some(url) = a.url.as_deref() {
                        validate_navigation_url(url)?;
                        args.insert("url".into(), serde_json::Value::String(url.to_string()));
                    }
                    let response = self.browser.call_upstream("browser_tabs", args).await?;
                    Ok(pages_from_tab_result(&self.browser, &response).await)
                }
                BrowserTabAction::Select | BrowserTabAction::Close => {
                    refresh_browser_tabs(&self.browser).await?;
                    let page_id = a.page_id.as_deref().ok_or_else(|| {
                        CoreError::new(
                            ErrorCode::InvalidArgument,
                            "browser_tabs select/close requires page_id",
                        )
                    })?;
                    let index = self.browser.page_index(page_id).await?;
                    let mut args = serde_json::Map::new();
                    args.insert(
                        "action".into(),
                        serde_json::Value::String(action_name.into()),
                    );
                    args.insert("index".into(), serde_json::json!(index));
                    let response = self.browser.call_upstream("browser_tabs", args).await?;
                    if matches!(a.action, BrowserTabAction::Close) {
                        self.browser.forget_page(page_id).await;
                    }
                    Ok(pages_from_tab_result(&self.browser, &response).await)
                }
            }
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_tabs",
            result,
            Some(serde_json::json!({
                "action": action_name,
                "page_id": a.page_id,
                "origin": origin,
            })),
        )
    }

    #[tool(
        description = "Navigate the active page to an http(s) URL, go back, go forward, or reload. file:// and non-http(s) schemes are rejected before Playwright is called.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserNavigationOutput>()
    )]
    async fn browser_navigate(
        &self,
        Parameters(a): Parameters<BrowserNavigateArgs>,
    ) -> CallToolResult {
        let action_name = match a.action {
            BrowserNavigationAction::Url => "url",
            BrowserNavigationAction::Back => "back",
            BrowserNavigationAction::Forward => "forward",
            BrowserNavigationAction::Reload => "reload",
        };
        let origin = a.url.as_deref().and_then(|url| redacted_origin(url).ok());
        let result = async {
            let mut args = serde_json::Map::new();
            let upstream = match a.action {
                BrowserNavigationAction::Url => {
                    let url = a.url.as_deref().ok_or_else(|| {
                        CoreError::new(
                            ErrorCode::InvalidArgument,
                            "browser_navigate action=url requires url",
                        )
                    })?;
                    validate_navigation_url(url)?;
                    args.insert("url".into(), serde_json::Value::String(url.to_string()));
                    "browser_navigate"
                }
                BrowserNavigationAction::Back => "browser_navigate_back",
                BrowserNavigationAction::Forward => "browser_navigate_forward",
                BrowserNavigationAction::Reload => "browser_reload",
            };
            let response = self.browser.call_upstream(upstream, args).await?;
            let (text, truncated) = truncate_utf8(call_result_text(&response), 256 * 1024);
            let pages = refresh_browser_tabs(&self.browser).await?;
            Ok::<_, CoreError>(BrowserNavigationOutput {
                text,
                truncated,
                pages,
            })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_navigate",
            result,
            Some(serde_json::json!({ "action": action_name, "origin": origin })),
        )
    }

    #[tool(
        description = "Search the web in the active browser using Google Search. The query is URL-encoded and is never written to the audit log.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserNavigationOutput>()
    )]
    async fn browser_search(&self, Parameters(a): Parameters<BrowserSearchArgs>) -> CallToolResult {
        let result = async {
            if a.query.trim().is_empty() {
                return Err(CoreError::new(
                    ErrorCode::InvalidArgument,
                    "browser_search query cannot be empty",
                ));
            }
            let mut url = url::Url::parse("https://www.google.com/search").expect("static URL");
            url.query_pairs_mut().append_pair("q", &a.query);
            let mut args = serde_json::Map::new();
            args.insert("url".into(), serde_json::Value::String(url.to_string()));
            let response = self.browser.call_upstream("browser_navigate", args).await?;
            let (text, truncated) = truncate_utf8(call_result_text(&response), 256 * 1024);
            let pages = refresh_browser_tabs(&self.browser).await?;
            Ok::<_, CoreError>(BrowserNavigationOutput {
                text,
                truncated,
                pages,
            })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_search",
            result,
            Some(serde_json::json!({ "query_bytes": a.query.len() })),
        )
    }

    #[tool(
        description = "Return a bounded semantic accessibility/DOM snapshot of the active page. Use returned target references for element interactions.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<BrowserSnapshotOutput>()
    )]
    async fn browser_snapshot(&self, Parameters(_): Parameters<EmptyArgs>) -> CallToolResult {
        let result = async {
            let response = self
                .browser
                .call_upstream("browser_snapshot", serde_json::Map::new())
                .await?;
            let (text, truncated) = truncate_utf8(
                call_result_text(&response),
                crate::browser::types::MAX_SNAPSHOT_BYTES,
            );
            let bytes = text.len();
            Ok::<_, CoreError>(BrowserSnapshotOutput {
                snapshot: text,
                bytes,
                truncated,
            })
        }
        .await;
        let detail = result
            .as_ref()
            .ok()
            .map(|v| serde_json::json!({"returned_bytes": v.bytes, "truncated": v.truncated}));
        browser_outcome(&self.core, "browser_snapshot", result, detail)
    }

    #[tool(
        description = "Click an element target from the current browser snapshot.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserTextTruncatedOutput>()
    )]
    async fn browser_click(&self, Parameters(a): Parameters<BrowserTargetArgs>) -> CallToolResult {
        let result = async {
            let mut args = serde_json::Map::new();
            args.insert("target".into(), serde_json::Value::String(a.ref_id.clone()));
            let r = self.browser.call_upstream("browser_click", args).await?;
            let (text, truncated) = truncate_utf8(call_result_text(&r), 64 * 1024);
            Ok::<_, CoreError>(BrowserTextTruncatedOutput { text, truncated })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_click",
            result,
            Some(serde_json::json!({"target":a.ref_id})),
        )
    }

    #[tool(
        description = "Fill/type text into an element target. Audit records length only and never the supplied text.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserTextOutput>()
    )]
    async fn browser_fill(&self, Parameters(a): Parameters<BrowserFillArgs>) -> CallToolResult {
        let result = async {
            let mut args = serde_json::Map::new();
            args.insert("target".into(), serde_json::Value::String(a.ref_id.clone()));
            args.insert("text".into(), serde_json::Value::String(a.text.clone()));
            args.insert("submit".into(), serde_json::Value::Bool(a.submit));
            args.insert("slowly".into(), serde_json::Value::Bool(a.slowly));
            let r = self.browser.call_upstream("browser_type", args).await?;
            Ok::<_, CoreError>(BrowserTextOutput {
                text: truncate_utf8(call_result_text(&r), 64 * 1024).0,
            })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_fill",
            result,
            Some(
                serde_json::json!({"target":a.ref_id,"text_bytes":a.text.len(),"submit":a.submit,"slowly":a.slowly}),
            ),
        )
    }

    #[tool(
        description = "Press a keyboard key in the active page, such as Enter or Escape.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserTextOutput>()
    )]
    async fn browser_press(&self, Parameters(a): Parameters<BrowserPressArgs>) -> CallToolResult {
        let result = async {
            if a.key.is_empty() || a.key.len() > 128 {
                return Err(CoreError::new(
                    ErrorCode::InvalidArgument,
                    "browser key must be 1 to 128 bytes",
                ));
            }
            let mut args = serde_json::Map::new();
            args.insert("key".into(), serde_json::Value::String(a.key.clone()));
            let r = self
                .browser
                .call_upstream("browser_press_key", args)
                .await?;
            Ok::<_, CoreError>(BrowserTextOutput {
                text: truncate_utf8(call_result_text(&r), 64 * 1024).0,
            })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_press",
            result,
            Some(serde_json::json!({"key":a.key})),
        )
    }

    #[tool(
        description = "Select one or more values in a select element target.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserTextOutput>()
    )]
    async fn browser_select(&self, Parameters(a): Parameters<BrowserSelectArgs>) -> CallToolResult {
        let result = async {
            if a.values.is_empty() || a.values.len() > 64 {
                return Err(CoreError::new(
                    ErrorCode::InvalidArgument,
                    "browser_select requires 1 to 64 values",
                ));
            }
            let mut args = serde_json::Map::new();
            args.insert("target".into(), serde_json::Value::String(a.ref_id.clone()));
            args.insert("values".into(), serde_json::json!(a.values));
            let r = self
                .browser
                .call_upstream("browser_select_option", args)
                .await?;
            Ok::<_, CoreError>(BrowserTextOutput {
                text: truncate_utf8(call_result_text(&r), 64 * 1024).0,
            })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_select",
            result,
            Some(serde_json::json!({"target":a.ref_id,"value_count":a.values.len()})),
        )
    }

    #[tool(
        description = "Hover an element target from the current browser snapshot.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserOkOutput>()
    )]
    async fn browser_hover(&self, Parameters(a): Parameters<BrowserTargetArgs>) -> CallToolResult {
        let result = async {
            let mut args = serde_json::Map::new();
            args.insert("target".into(), serde_json::Value::String(a.ref_id.clone()));
            self.browser.call_upstream("browser_hover", args).await?;
            Ok::<_, CoreError>(BrowserOkOutput { ok: true })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_hover",
            result,
            Some(serde_json::json!({"target":a.ref_id})),
        )
    }

    #[tool(
        description = "Drag from one semantic element target to another.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserOkOutput>()
    )]
    async fn browser_drag(&self, Parameters(a): Parameters<BrowserDragArgs>) -> CallToolResult {
        let result = async {
            let mut args = serde_json::Map::new();
            args.insert(
                "startTarget".into(),
                serde_json::Value::String(a.start_ref.clone()),
            );
            args.insert(
                "endTarget".into(),
                serde_json::Value::String(a.end_ref.clone()),
            );
            self.browser.call_upstream("browser_drag", args).await?;
            Ok::<_, CoreError>(BrowserOkOutput { ok: true })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_drag",
            result,
            Some(serde_json::json!({"start_ref":a.start_ref,"end_ref":a.end_ref})),
        )
    }

    #[tool(
        description = "Wait for text, element visibility, URL, load state, or a bounded delay. Pause/disable is checked while polling.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<BrowserOkOutput>()
    )]
    async fn browser_wait(&self, Parameters(a): Parameters<BrowserWaitArgs>) -> CallToolResult {
        let kind = match &a.condition {
            BrowserWaitCondition::TextPresent { .. } => "text_present",
            BrowserWaitCondition::TextAbsent { .. } => "text_absent",
            BrowserWaitCondition::RefVisible { .. } => "ref_visible",
            BrowserWaitCondition::RefHidden { .. } => "ref_hidden",
            BrowserWaitCondition::UrlContains { .. } => "url_contains",
            BrowserWaitCondition::LoadState { .. } => "load_state",
            BrowserWaitCondition::Delay { .. } => "delay",
        };
        let result = browser_wait_internal(&self.browser, a.condition, a.timeout_ms).await;
        browser_outcome(
            &self.core,
            "browser_wait",
            result,
            Some(serde_json::json!({"kind":kind,"timeout_ms":a.timeout_ms})),
        )
    }

    #[tool(
        description = "Accept or dismiss the active browser dialog; optional prompt text is never written to audit.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserOkOutput>()
    )]
    async fn browser_dialog(&self, Parameters(a): Parameters<BrowserDialogArgs>) -> CallToolResult {
        let result = async {
            let mut args = serde_json::Map::new();
            args.insert("accept".into(), serde_json::Value::Bool(a.accept));
            if let Some(text) = a.prompt_text.clone() {
                args.insert("promptText".into(), serde_json::Value::String(text));
            }
            self.browser
                .call_upstream("browser_handle_dialog", args)
                .await?;
            Ok::<_, CoreError>(BrowserOkOutput { ok: true })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_dialog",
            result,
            Some(
                serde_json::json!({"accept":a.accept,"prompt_bytes":a.prompt_text.as_ref().map(|s|s.len()).unwrap_or(0)}),
            ),
        )
    }

    #[tool(
        description = "Resize the active browser viewport to exact CSS pixel dimensions, bounded to 1..8192 on each edge.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserViewportOutput>()
    )]
    async fn browser_set_viewport(
        &self,
        Parameters(a): Parameters<BrowserViewportArgs>,
    ) -> CallToolResult {
        let result = async {
            validate_viewport(a.width, a.height)?;
            let mut args = serde_json::Map::new();
            args.insert("width".into(), serde_json::json!(a.width));
            args.insert("height".into(), serde_json::json!(a.height));
            self.browser.call_upstream("browser_resize", args).await?;
            Ok::<_, CoreError>(BrowserViewportOutput {
                width: a.width,
                height: a.height,
            })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_set_viewport",
            result,
            Some(serde_json::json!({"width":a.width,"height":a.height})),
        )
    }

    #[tool(
        description = "Apply a Playwright device preset or explicit mobile/touch emulation. A running browser is restarted into a new session while reusing its dedicated engine profile.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserSessionStatus>()
    )]
    async fn browser_emulate_device(
        &self,
        Parameters(a): Parameters<BrowserEmulateArgs>,
    ) -> CallToolResult {
        let result = async {
            let emulation = match a.emulation {
                BrowserEmulationRequest::Preset { name, orientation } => {
                    let d = self.browser.device_descriptor(&name).await?;
                    let vp = d
                        .get("viewport")
                        .and_then(|v| v.as_object())
                        .ok_or_else(|| {
                            CoreError::new(
                                ErrorCode::BrowserFeatureUnsupported,
                                "device preset has no viewport",
                            )
                        })?;
                    let mut width = vp.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    let mut height = vp.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    let landscape = matches!(orientation, Some(BrowserOrientation::Landscape));
                    if landscape && height > width {
                        std::mem::swap(&mut width, &mut height);
                    }
                    if matches!(orientation, Some(BrowserOrientation::Portrait)) && width > height {
                        std::mem::swap(&mut width, &mut height);
                    }
                    validate_viewport(width, height)?;
                    crate::browser::types::BrowserEmulation {
                        width,
                        height,
                        device_scale_factor: d
                            .get("deviceScaleFactor")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(1.0),
                        is_mobile: d.get("isMobile").and_then(|v| v.as_bool()).unwrap_or(false),
                        has_touch: d.get("hasTouch").and_then(|v| v.as_bool()).unwrap_or(false),
                        user_agent: d
                            .get("userAgent")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        landscape,
                    }
                }
                BrowserEmulationRequest::Explicit {
                    width,
                    height,
                    device_scale_factor,
                    is_mobile,
                    has_touch,
                    user_agent,
                    orientation,
                } => {
                    validate_viewport(width, height)?;
                    if !(0.1..=10.0).contains(&device_scale_factor) || user_agent.len() > 4096 {
                        return Err(CoreError::new(
                            ErrorCode::InvalidArgument,
                            "invalid browser emulation scale or user-agent",
                        ));
                    }
                    crate::browser::types::BrowserEmulation {
                        width,
                        height,
                        device_scale_factor,
                        is_mobile,
                        has_touch,
                        user_agent: Some(user_agent),
                        landscape: matches!(orientation, BrowserOrientation::Landscape),
                    }
                }
            };
            self.browser.restart_for_emulation(emulation).await
        }
        .await;
        let detail=result.as_ref().ok().and_then(|s|s.emulation.as_ref()).map(|e|serde_json::json!({"width":e.width,"height":e.height,"device_scale_factor":e.device_scale_factor,"is_mobile":e.is_mobile,"has_touch":e.has_touch}));
        browser_outcome(&self.core, "browser_emulate_device", result, detail)
    }

    #[tool(
        description = "Capture the active page as PNG (viewport, full page, or element). Returns an MCP image block and safe metadata; independent of desktop Screen Capture permission.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<BrowserScreenshotMetadata>()
    )]
    async fn browser_screenshot(
        &self,
        Parameters(a): Parameters<BrowserScreenshotArgs>,
    ) -> CallToolResult {
        let mode = a.mode;
        let captured = async {
            if matches!(mode, BrowserScreenshotMode::Element) && a.ref_id.is_none() {
                return Err(CoreError::new(
                    ErrorCode::InvalidArgument,
                    "element screenshot requires ref",
                ));
            }
            if !matches!(mode, BrowserScreenshotMode::Element) && a.ref_id.is_some() {
                return Err(CoreError::new(
                    ErrorCode::InvalidArgument,
                    "ref is only valid for element screenshots",
                ));
            }
            let mut args = serde_json::Map::new();
            args.insert("type".into(), serde_json::Value::String("png".into()));
            args.insert("scale".into(), serde_json::Value::String("css".into()));
            args.insert(
                "fullPage".into(),
                serde_json::Value::Bool(matches!(mode, BrowserScreenshotMode::FullPage)),
            );
            if matches!(mode, BrowserScreenshotMode::Element) {
                if let Some(target) = a.ref_id.clone() {
                    args.insert("target".into(), serde_json::Value::String(target));
                }
            }
            let response = self
                .browser
                .call_upstream("browser_take_screenshot", args)
                .await?;
            browser_screenshot_payload(&response, mode, a.max_edge)
        }
        .await;
        let audit_detail=captured.as_ref().ok().map(|(_,m)|serde_json::json!({"mode":m.mode,"returned_width":m.returned_width,"returned_height":m.returned_height,"encoded_bytes":m.encoded_bytes,"sha256":m.sha256}));
        if let Err(audit_error) = crate::browser::audit::record_browser_call(
            &self.core,
            "browser_screenshot",
            captured.as_ref().err(),
            audit_detail,
        ) {
            return err_result(&audit_error);
        }
        match captured {
            Ok((bytes, metadata)) => screenshot_tool_result(bytes, &metadata),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Read bounded browser console messages. Message text is returned to the caller but audit stores only level/count metadata.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<BrowserConsoleOutput>()
    )]
    async fn browser_console(
        &self,
        Parameters(a): Parameters<BrowserConsoleArgs>,
    ) -> CallToolResult {
        let result = async {
            let limit = bounded_event_limit(a.limit)?;
            let level = a.level.clone().unwrap_or_else(|| "info".into());
            if !matches!(level.as_str(), "error" | "warning" | "info" | "debug") {
                return Err(CoreError::new(
                    ErrorCode::InvalidArgument,
                    "browser console level must be error, warning, info, or debug",
                ));
            }
            let mut args = serde_json::Map::new();
            args.insert("level".into(), serde_json::Value::String(level.clone()));
            args.insert("all".into(), serde_json::Value::Bool(true));
            let response = self
                .browser
                .call_upstream("browser_console_messages", args)
                .await?;
            let raw = call_result_text(&response);
            let (text, next_cursor) =
                paginate_lines(strip_console_header(&raw), a.cursor.as_deref(), limit)?;
            let count = text.lines().count();
            let levels = console_level_histogram(&text);
            Ok::<_, CoreError>(BrowserConsoleOutput {
                level,
                count,
                levels,
                messages: text,
                next_cursor,
            })
        }
        .await;
        let detail = result
            .as_ref()
            .ok()
            .map(|v| serde_json::json!({"count":v.count,"levels":v.levels,"cursor":v.next_cursor}));
        browser_outcome(&self.core, "browser_console", result, detail)
    }

    #[tool(
        description = "Read bounded browser network request metadata. The optional filter is a regular expression matched against request URLs (e.g. \"/api/.*user\"). Sensitive authorization/cookie/API-key header lines are redacted and response bodies are never fetched.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<BrowserNetworkOutput>()
    )]
    async fn browser_network(
        &self,
        Parameters(a): Parameters<BrowserNetworkArgs>,
    ) -> CallToolResult {
        let result = async {
            let limit = bounded_event_limit(a.limit)?;
            let mut args = serde_json::Map::new();
            args.insert("static".into(), serde_json::Value::Bool(a.include_static));
            if let Some(filter) = a.filter.clone() {
                // Pre-flight guard: the filter is forwarded upstream as a JavaScript
                // regular expression; bound its length and reject control characters
                // before it leaves this process.
                if filter.len() > 4096 || filter.bytes().any(|byte| byte < 0x20) {
                    return Err(CoreError::new(
                        ErrorCode::InvalidArgument,
                        "browser network filter must be a regular expression of at most 4096 bytes without control characters",
                    ));
                }
                args.insert("filter".into(), serde_json::Value::String(filter));
            }
            let response = self
                .browser
                .call_upstream("browser_network_requests", args)
                .await?;
            let safe = redact_network_text(call_result_text(&response));
            let (text, next_cursor) = paginate_lines(&safe, a.cursor.as_deref(), limit)?;
            let count = text.lines().count();
            Ok::<_, CoreError>(BrowserNetworkOutput {
                count,
                requests: text,
                next_cursor,
            })
        }
        .await;
        let detail = result
            .as_ref()
            .ok()
            .map(|v| serde_json::json!({"count":v.count,"cursor":v.next_cursor}));
        browser_outcome(&self.core, "browser_network", result, detail)
    }

    #[tool(
        description = "Inspect one element target with app-owned DOM/computed-style JavaScript. Sensitive value/token/password-like attributes are redacted.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<BrowserDomOutput>()
    )]
    async fn browser_dom(&self, Parameters(a): Parameters<BrowserDomArgs>) -> CallToolResult {
        let result = async {
            let mut args = serde_json::Map::new();
            args.insert("target".into(), serde_json::Value::String(a.ref_id.clone()));
            args.insert(
                "function".into(),
                serde_json::Value::String(DOM_INSPECT_FUNCTION.into()),
            );
            let response = self.browser.call_upstream("browser_evaluate", args).await?;
            let (text, truncated) = truncate_utf8(call_result_text(&response), 64 * 1024);
            Ok::<_, CoreError>(BrowserDomOutput {
                target: a.ref_id.clone(),
                result: text,
                truncated,
            })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_dom",
            result,
            Some(serde_json::json!({"target":a.ref_id})),
        )
    }

    #[tool(
        description = "Evaluate JavaScript inside the active page context, optionally against an element target. This never routes through Playwright process-level code execution.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<BrowserEvaluateOutput>()
    )]
    async fn browser_evaluate(
        &self,
        Parameters(a): Parameters<BrowserEvaluateArgs>,
    ) -> CallToolResult {
        let result = async {
            if a.expression.is_empty() || a.expression.len() > 256 * 1024 {
                return Err(CoreError::new(
                    ErrorCode::InvalidArgument,
                    "browser evaluate expression must be 1..262144 bytes",
                ));
            }
            let mut args = serde_json::Map::new();
            args.insert(
                "function".into(),
                serde_json::Value::String(a.expression.clone()),
            );
            if let Some(target) = a.ref_id.clone() {
                args.insert("target".into(), serde_json::Value::String(target));
            }
            let response = self.browser.call_upstream("browser_evaluate", args).await?;
            let (text, truncated) = truncate_utf8(call_result_text(&response), 256 * 1024);
            Ok::<_, CoreError>(BrowserEvaluateOutput {
                result: text,
                truncated,
            })
        }
        .await;
        browser_outcome(
            &self.core,
            "browser_evaluate",
            result,
            Some(
                serde_json::json!({"expression_bytes":a.expression.len(),"has_target":a.ref_id.is_some()}),
            ),
        )
    }

    #[tool(
        description = "Read browser performance timing/resource data. Chromium-family engines additionally merge app-owned CDP Performance.getMetrics output; Firefox returns page metrics without CDP.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<BrowserPerformanceOutput>()
    )]
    async fn browser_performance(&self, Parameters(_): Parameters<EmptyArgs>) -> CallToolResult {
        let result = async {
            let mut args = serde_json::Map::new();
            args.insert(
                "function".into(),
                serde_json::Value::String(PERFORMANCE_FUNCTION.into()),
            );
            let page = self.browser.call_upstream("browser_evaluate", args).await?;
            let (page_text, page_truncated) = truncate_utf8(call_result_text(&page), 256 * 1024);
            let engine = self.browser.active_engine().await.ok_or_else(|| {
                CoreError::new(ErrorCode::BrowserNotRunning, "browser is not running")
            })?;
            let (cdp_supported, cdp) = if engine.is_chromium_family() {
                (
                    true,
                    Some(self.browser.chromium_performance_metrics().await?),
                )
            } else {
                (false, None)
            };
            Ok::<_, CoreError>(BrowserPerformanceOutput {
                page: page_text,
                page_truncated,
                cdp_supported,
                cdp,
            })
        }
        .await;
        let detail=result.as_ref().ok().map(|v|serde_json::json!({"cdp_supported":v.cdp_supported,"page_truncated":v.page_truncated}));
        browser_outcome(&self.core, "browser_performance", result, detail)
    }

    #[tool(
        description = "Start, stop, or read status of Playwright tracing for the active browser session. Trace artifacts stay inside the session sandbox and are returned only as opaque metadata.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserTraceOutput>()
    )]
    async fn browser_trace(&self, Parameters(a): Parameters<BrowserTraceArgs>) -> CallToolResult {
        let action = match a.action {
            BrowserTraceAction::Start => "start",
            BrowserTraceAction::Stop => "stop",
            BrowserTraceAction::Status => "status",
        };
        let result = async {
            self.browser.guard_page_operation().await?;
            match a.action {
                BrowserTraceAction::Status => Ok::<_, CoreError>(BrowserTraceOutput {
                    tracing: self.browser.is_tracing().await,
                    artifact_id: None,
                    files: None,
                    bytes: None,
                }),
                BrowserTraceAction::Start => {
                    if self.browser.is_tracing().await {
                        return Err(CoreError::new(
                            ErrorCode::InvalidArgument,
                            "browser trace is already running",
                        ));
                    }
                    self.browser
                        .call_upstream("browser_start_tracing", serde_json::Map::new())
                        .await?;
                    self.browser.set_tracing(true).await;
                    Ok(BrowserTraceOutput {
                        tracing: true,
                        artifact_id: None,
                        files: None,
                        bytes: None,
                    })
                }
                BrowserTraceAction::Stop => {
                    if !self.browser.is_tracing().await {
                        return Err(CoreError::new(
                            ErrorCode::InvalidArgument,
                            "browser trace is not running",
                        ));
                    }
                    self.browser
                        .call_upstream("browser_stop_tracing", serde_json::Map::new())
                        .await?;
                    self.browser.set_tracing(false).await;
                    let trace_dir = self.browser.active_artifact_dir().await?.join("traces");
                    let mut files = 0usize;
                    let mut total = 0u64;
                    let mut records: Vec<(String, String)> = Vec::new();
                    let mut stack = vec![trace_dir.clone()];
                    while let Some(dir) = stack.pop() {
                        for entry in std::fs::read_dir(&dir).map_err(CoreError::io)? {
                            let entry = entry.map_err(CoreError::io)?;
                            let p = entry.path();
                            if p.is_dir() {
                                stack.push(p);
                            } else if p.is_file() {
                                let rel = p
                                    .strip_prefix(&trace_dir)
                                    .map_err(|_| {
                                        CoreError::new(
                                            ErrorCode::BrowserArtifactFailed,
                                            "browser trace escaped the active trace directory",
                                        )
                                    })?
                                    .to_string_lossy()
                                    .replace('\\', "/");
                                let bytes = std::fs::read(&p).map_err(CoreError::io)?;
                                total = total.saturating_add(bytes.len() as u64);
                                files += 1;
                                records.push((rel, lc_core::ops::sha256_hex(&bytes)));
                            }
                        }
                    }
                    // Brief, opaque stop metadata: one artifact id fingerprinting the whole
                    // trace set (sorted rel-path + per-file sha256), a file count, and total
                    // bytes. No filenames, per-file hashes, or paths are returned.
                    records.sort();
                    let fingerprint: String = records
                        .iter()
                        .map(|(rel, hash)| format!("{rel}:{hash}\n"))
                        .collect();
                    let artifact_id = format!(
                        "a-{}",
                        &lc_core::ops::sha256_hex(fingerprint.as_bytes())[..16]
                    );
                    Ok(BrowserTraceOutput {
                        tracing: false,
                        artifact_id: Some(artifact_id),
                        files: Some(files),
                        bytes: Some(total),
                    })
                }
            }
        }
        .await;
        let detail = Some(serde_json::json!({"action":action}));
        browser_outcome(&self.core, "browser_trace", result, detail)
    }

    #[tool(
        description = "Report Visible Debug DevTools support for the current browser. DevTools UI is available only when configured Visible Debug on Chromium/Chrome/Edge; this tool never launches a second browser.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<BrowserDevtoolsOutput>()
    )]
    async fn browser_devtools(&self, Parameters(_): Parameters<EmptyArgs>) -> CallToolResult {
        let result = async {
            self.browser.guard_page_operation().await?;
            let launch = self.browser.active_launch().await.ok_or_else(|| {
                CoreError::new(ErrorCode::BrowserNotRunning, "browser is not running")
            })?;
            if launch.run_mode != crate::browser::types::BrowserRunMode::VisibleDebug {
                return Err(CoreError::new(
                    ErrorCode::BrowserFeatureUnsupportedInMode,
                    "visible DevTools requires Visible Debug run mode",
                ));
            }
            if !launch.engine.is_chromium_family() {
                return Err(CoreError::new(
                    ErrorCode::BrowserFeatureUnsupported,
                    "visible DevTools is supported only by Chromium-family engines",
                ));
            }
            let auto_open = self.browser.devtools_auto_open().await.unwrap_or(false);
            if !auto_open {
                return Err(CoreError::new(
                    ErrorCode::BrowserFeatureUnsupported,
                    "DevTools auto-open was not part of the browser launch",
                ));
            }
            Ok::<_, CoreError>(BrowserDevtoolsOutput {
                supported: true,
                mode: "visible_debug".to_string(),
                engine: launch.engine,
                auto_open,
            })
        }
        .await;
        browser_outcome(&self.core, "browser_devtools", result, None)
    }

    #[tool(
        description = "Stage approved workspace files into the active browser sandbox and upload them through Playwright. Upstream never receives original workspace paths.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserUploadOutput>()
    )]
    async fn browser_upload(&self, Parameters(a): Parameters<BrowserUploadArgs>) -> CallToolResult {
        let sources: Vec<serde_json::Value> = a
            .sources
            .iter()
            .map(|source| match source {
                BrowserUploadSource::Workspace { path } => {
                    serde_json::json!({"kind":"workspace","path":path})
                }
                BrowserUploadSource::Artifact { .. } => serde_json::json!({"kind":"artifact"}),
            })
            .collect();
        let result = async {
            let artifact = self.browser.active_artifact_dir().await?;
            let staged = crate::browser::files::stage_uploads(
                &self.core,
                &artifact,
                a.workspace_id.as_deref(),
                &a.sources,
            )?;
            let total = staged
                .iter()
                .filter_map(|p| std::fs::metadata(p).ok().map(|m| m.len()))
                .sum::<u64>();
            let mut args = serde_json::Map::new();
            args.insert(
                "paths".into(),
                serde_json::Value::Array(
                    staged
                        .iter()
                        .map(|p| serde_json::Value::String(p.to_string_lossy().to_string()))
                        .collect(),
                ),
            );
            let upload = self
                .browser
                .call_upstream("browser_file_upload", args)
                .await;
            // Best-effort: the staged copies have served their purpose once the
            // upstream call returns (success or failure); remove them so the
            // session sandbox does not accumulate duplicate upload bytes
            // (final-review Minor 5).
            for path in &staged {
                let _ = std::fs::remove_file(path);
            }
            upload?;
            Ok::<_, CoreError>(BrowserUploadOutput {
                file_count: staged.len(),
                bytes: total,
            })
        }
        .await;
        let mut detail = serde_json::json!({
            "workspace_id": a.workspace_id,
            "sources": sources,
            "file_count": a.sources.len(),
        });
        if let Ok(value) = &result {
            detail["bytes"] = serde_json::json!(value.bytes);
        }
        browser_outcome(&self.core, "browser_upload", result, Some(detail))
    }

    #[tool(
        description = "List completed downloads in the active browser session sandbox using opaque download ids; absolute filesystem paths are never returned.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<Vec<BrowserDownloadInfo>>()
    )]
    async fn browser_downloads(&self, Parameters(_): Parameters<EmptyArgs>) -> CallToolResult {
        let result = async {
            let artifact = self.browser.active_artifact_dir().await?;
            crate::browser::files::list_downloads(&artifact)
        }
        .await;
        let detail = result
            .as_ref()
            .ok()
            .map(|v| serde_json::json!({"count":v.len()}));
        browser_outcome(&self.core, "browser_downloads", result, detail)
    }

    #[tool(
        description = "Copy one opaque browser download into an approved workspace-relative destination. Requires workspace Write permission and canonical path confinement.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserSavedDownloadOutput>()
    )]
    async fn browser_save_download_to_workspace(
        &self,
        Parameters(a): Parameters<BrowserSaveDownloadArgs>,
    ) -> CallToolResult {
        let result = async {
            let artifact = self.browser.active_artifact_dir().await?;
            crate::browser::files::save_download_to_workspace(
                &self.core,
                &artifact,
                &a.workspace_id,
                &a.download_id,
                &a.path,
            )
        }
        .await;
        let mut detail = serde_json::json!({"workspace_id":a.workspace_id,"download_id":a.download_id,"path":a.path});
        if let Ok(value) = &result {
            detail["bytes"] = serde_json::json!(value.bytes);
            detail["sha256"] = serde_json::json!(value.sha256);
        }
        browser_outcome(
            &self.core,
            "browser_save_download_to_workspace",
            result,
            Some(detail),
        )
    }

    #[tool(
        description = "Run up to 20 safe browser steps sequentially using the same Local Workspaces validation/policy paths. Stops on first failure and never replays a side-effecting step after process loss.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<BrowserRunStepsOutput>()
    )]
    async fn browser_run_steps(
        &self,
        Parameters(a): Parameters<BrowserRunStepsArgs>,
    ) -> CallToolResult {
        let step_count = a.steps.len();
        let types: Vec<&'static str> = a
            .steps
            .iter()
            .map(|s| match s {
                BrowserStep::Navigate { .. } => "navigate",
                BrowserStep::Snapshot => "snapshot",
                BrowserStep::Click { .. } => "click",
                BrowserStep::Fill { .. } => "fill",
                BrowserStep::Press { .. } => "press",
                BrowserStep::Select { .. } => "select",
                BrowserStep::Wait { .. } => "wait",
                BrowserStep::SetViewport { .. } => "set_viewport",
                BrowserStep::Screenshot { .. } => "screenshot",
            })
            .collect();
        let mut step_failure: Option<CoreError> = None;
        let result=async {
            validate_batch_step_count(step_count)?;
            self.browser.guard_page_operation().await?;
            let mut results=Vec::with_capacity(step_count);
            for (index,step) in a.steps.into_iter().enumerate(){
                let one:Result<serde_json::Value,CoreError>=async {
                    match step {
                        BrowserStep::Navigate{url}=>{validate_navigation_url(&url)?;let mut args=serde_json::Map::new();args.insert("url".into(),serde_json::Value::String(url));let r=self.browser.call_upstream("browser_navigate",args).await?;Ok(serde_json::json!({"text":truncate_utf8(call_result_text(&r),64*1024).0}))}
                        BrowserStep::Snapshot=>{let r=self.browser.call_upstream("browser_snapshot",serde_json::Map::new()).await?;let (text,truncated)=truncate_utf8(call_result_text(&r),crate::browser::types::MAX_SNAPSHOT_BYTES);Ok(serde_json::json!({"snapshot":text,"truncated":truncated}))}
                        BrowserStep::Click{ref_id}=>{let mut args=serde_json::Map::new();args.insert("target".into(),serde_json::Value::String(ref_id));self.browser.call_upstream("browser_click",args).await?;Ok(serde_json::json!({"ok":true}))}
                        BrowserStep::Fill{ref_id,text}=>{let mut args=serde_json::Map::new();args.insert("target".into(),serde_json::Value::String(ref_id));args.insert("text".into(),serde_json::Value::String(text));args.insert("submit".into(),serde_json::Value::Bool(false));args.insert("slowly".into(),serde_json::Value::Bool(false));self.browser.call_upstream("browser_type",args).await?;Ok(serde_json::json!({"ok":true}))}
                        BrowserStep::Press{key}=>{let mut args=serde_json::Map::new();args.insert("key".into(),serde_json::Value::String(key));self.browser.call_upstream("browser_press_key",args).await?;Ok(serde_json::json!({"ok":true}))}
                        BrowserStep::Select{ref_id,values}=>{let mut args=serde_json::Map::new();args.insert("target".into(),serde_json::Value::String(ref_id));args.insert("values".into(),serde_json::json!(values));self.browser.call_upstream("browser_select_option",args).await?;Ok(serde_json::json!({"ok":true}))}
                        BrowserStep::Wait{condition,timeout_ms}=>{let ok=browser_wait_internal(&self.browser,condition,timeout_ms).await?;Ok(serde_json::to_value(ok).unwrap_or_default())}
                        BrowserStep::SetViewport{width,height}=>{validate_viewport(width,height)?;let mut args=serde_json::Map::new();args.insert("width".into(),serde_json::json!(width));args.insert("height".into(),serde_json::json!(height));self.browser.call_upstream("browser_resize",args).await?;Ok(serde_json::json!({"width":width,"height":height}))}
                        BrowserStep::Screenshot{mode,ref_id,max_edge}=>{if matches!(mode,BrowserScreenshotMode::Element)&&ref_id.is_none(){return Err(CoreError::new(ErrorCode::InvalidArgument,"element screenshot requires ref"));} else if !matches!(mode,BrowserScreenshotMode::Element)&&ref_id.is_some(){return Err(CoreError::new(ErrorCode::InvalidArgument,"ref is only valid for element screenshots"));}let mut args=serde_json::Map::new();args.insert("type".into(),serde_json::Value::String("png".into()));args.insert("scale".into(),serde_json::Value::String("css".into()));args.insert("fullPage".into(),serde_json::Value::Bool(matches!(mode,BrowserScreenshotMode::FullPage)));if matches!(mode,BrowserScreenshotMode::Element){if let Some(t)=ref_id{args.insert("target".into(),serde_json::Value::String(t));}}let r=self.browser.call_upstream("browser_take_screenshot",args).await?;let (_bytes,metadata)=browser_screenshot_payload(&r,mode,max_edge)?;Ok(serde_json::json!({"metadata":metadata}))}
                    }
                }.await;
                match one {
                    Ok(v)=>results.push(serde_json::json!({"index":index,"ok":true,"result":v})),
                    Err(e)=>{
                        step_failure=Some(e.clone());
                        return Ok::<_,CoreError>(BrowserRunStepsOutput {
                            ok:false,
                            results,
                            failed_index:Some(index),
                            status:Some("failed".to_string()),
                            error:Some(BrowserStepError { code: e.code.as_str().to_string(), message: e.message }),
                        });
                    }
                }
            }
            Ok::<_,CoreError>(BrowserRunStepsOutput { ok:true, results, failed_index:None, status:None, error:None })
        }.await;
        // A failed step must never audit as allow: record failed_index/status
        // and the error decision while still returning per-step results to the
        // caller (stop-on-first-failure, no replay).
        let audit_error: Option<CoreError> = match &result {
            Ok(_) if step_failure.is_some() => step_failure.clone(),
            Ok(_) => None,
            Err(error) => Some(error.clone()),
        };
        let detail = Some(match &result {
            Ok(_) if step_failure.is_some() => serde_json::json!({
                "step_count": step_count,
                "step_types": types,
                "status": "failed",
                "failed_index": result.as_ref().ok().and_then(|v| v.failed_index),
                "error": step_failure.as_ref().map(|e| serde_json::json!({"code": e.code.as_str(), "message": e.message})),
            }),
            Ok(_) => serde_json::json!({
                "step_count": step_count,
                "step_types": types,
                "status": "ok",
                "failed_index": null,
            }),
            Err(_) => serde_json::json!({
                "step_count": step_count,
                "step_types": types,
                "status": "error",
            }),
        });
        if let Err(audit_error) = crate::browser::audit::record_browser_call(
            &self.core,
            "browser_run_steps",
            audit_error.as_ref(),
            detail,
        ) {
            return err_result(&audit_error);
        }
        match result {
            Ok(value) => ok_structured(&value),
            Err(error) => err_result(&error),
        }
    }

    #[tool(
        description = "Read the live Local Workspaces agent/privacy policy before acting. Returns Agent mode, screen-capture authorization/scope, visual-verification preference, Pause state, and unrestricted-access state. Pause remains authoritative and denies the call when active.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<AgentPolicyOutput>()
    )]
    fn get_agent_policy(&self, Parameters(_): Parameters<EmptyArgs>) -> CallToolResult {
        let paused = self.core.pause.is_paused();
        let audited: Result<AgentPolicyOutput, CoreError> = if paused {
            Err(CoreError::new(
                ErrorCode::AccessPaused,
                "access is paused by the user",
            ))
        } else {
            let snapshot = self.agent_policy.snapshot();
            Ok(AgentPolicyOutput {
                mode: snapshot.mode,
                paused,
                unrestricted_access: self.core.unrestricted.get(),
                screen_capture_enabled: snapshot.screen_capture_enabled,
                capture_scope: snapshot.capture_scope,
                visual_verification: snapshot.visual_verification,
            })
        };
        let decision = match &audited {
            Ok(_) => "allow",
            Err(e) if e.code == ErrorCode::AccessPaused => "deny",
            Err(_) => "error",
        };
        let detail = audited.as_ref().ok().map(|p| {
            serde_json::json!({
                "mode": p.mode,
                "screen_capture_enabled": p.screen_capture_enabled,
                "capture_scope": p.capture_scope,
                "visual_verification": p.visual_verification,
                "unrestricted": p.unrestricted_access,
            })
        });
        if let Err(e) = lc_core::audit::record_simple(
            &self.core.db,
            "mcp",
            Some("get_agent_policy"),
            None,
            "get_agent_policy",
            decision,
            audited.as_ref().err().map(|e| e.code.as_str()),
            None,
            detail,
        ) {
            return err_result(&e);
        }
        match audited {
            Ok(policy) => ok_structured(&policy),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Update only non-escalating Local Workspaces Agent preferences. Agent mode and visual-verification preference may change freely; screen capture may be disabled and capture scope may be narrowed to windows_only. This tool cannot enable screen capture, broaden capture scope, enable unrestricted access, grant workspace permissions, change connection identity, or modify secrets. Those privilege increases require the user to use desktop Settings.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<UpdateAgentSettingsOutput>()
    )]
    fn update_agent_settings(
        &self,
        Parameters(a): Parameters<crate::settings::AgentSettingsPatch>,
    ) -> CallToolResult {
        let mut requested_fields = Vec::new();
        if a.agent_mode.is_some() {
            requested_fields.push("agent_mode");
        }
        if a.screen_capture_enabled.is_some() {
            requested_fields.push("screen_capture_enabled");
        }
        if a.capture_scope.is_some() {
            requested_fields.push("capture_scope");
        }
        if a.visual_verification.is_some() {
            requested_fields.push("visual_verification");
        }

        let changed: Result<crate::settings::AgentSettingsChange, CoreError> =
            if self.core.pause.is_paused() {
                Err(CoreError::new(
                    ErrorCode::AccessPaused,
                    "access is paused by the user",
                ))
            } else {
                self.agent_settings.update(a)
            };

        let decision = match &changed {
            Ok(_) => "allow",
            Err(e)
                if matches!(
                    e.code,
                    ErrorCode::AccessPaused | ErrorCode::SettingsElevationRequiresUser
                ) =>
            {
                "deny"
            }
            Err(_) => "error",
        };

        let detail = match &changed {
            Ok(change) => Some(serde_json::json!({
                "changed_fields": change.changed_fields,
                "mode": change.current.agent_mode,
                "screen_capture_enabled": change.current.screen_capture_enabled,
                "capture_scope": change.current.capture_scope,
                "visual_verification": change.current.visual_verification,
            })),
            Err(_) => Some(serde_json::json!({
                "requested_fields": requested_fields,
            })),
        };

        if let Err(audit_error) = lc_core::audit::record_simple(
            &self.core.db,
            "mcp",
            Some("update_agent_settings"),
            None,
            "update_agent_settings",
            decision,
            changed.as_ref().err().map(|e| e.code.as_str()),
            None,
            detail,
        ) {
            if let Ok(change) = &changed {
                if !change.changed_fields.is_empty() {
                    let _ = self.agent_settings.restore(&change.previous);
                }
            }
            return err_result(&audit_error);
        }

        match changed {
            Ok(change) => ok_structured(&UpdateAgentSettingsOutput {
                mode: change.current.agent_mode,
                screen_capture_enabled: change.current.screen_capture_enabled,
                capture_scope: change.current.capture_scope,
                visual_verification: change.current.visual_verification,
                changed_fields: change
                    .changed_fields
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            }),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "List screen-capture targets allowed by the live agent policy. Requires explicit screen-capture enablement; Pause always wins. When capture scope is windows_only, full displays are omitted.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<Vec<CaptureTarget>>()
    )]
    fn list_capture_targets(&self, Parameters(_): Parameters<EmptyArgs>) -> CallToolResult {
        let snapshot = self.agent_policy.snapshot();
        let audited: Result<Vec<CaptureTarget>, CoreError> = (|| {
            if self.core.pause.is_paused() {
                return Err(CoreError::new(
                    ErrorCode::AccessPaused,
                    "access is paused by the user",
                ));
            }
            if !snapshot.screen_capture_enabled {
                return Err(CoreError::new(
                    ErrorCode::ScreenCaptureDisabled,
                    "screen capture is disabled by the user",
                ));
            }
            let targets = self.capture_provider.list_targets()?;
            Ok(filter_targets_for_scope(targets, snapshot.capture_scope))
        })();
        let decision = match &audited {
            Ok(_) => "allow",
            Err(e)
                if matches!(
                    e.code,
                    ErrorCode::AccessPaused | ErrorCode::ScreenCaptureDisabled
                ) =>
            {
                "deny"
            }
            Err(_) => "error",
        };
        let detail = audited.as_ref().ok().map(|targets| {
            serde_json::json!({
                "count": targets.len(),
                "capture_scope": snapshot.capture_scope,
            })
        });
        if let Err(e) = lc_core::audit::record_simple(
            &self.core.db,
            "mcp",
            Some("list_capture_targets"),
            None,
            "list_capture_targets",
            decision,
            audited.as_ref().err().map(|e| e.code.as_str()),
            None,
            detail,
        ) {
            return err_result(&e);
        }
        match audited {
            Ok(targets) => ok_structured(&targets),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Capture one PNG frame from a target returned by list_capture_targets. Requires explicit screen-capture enablement; Pause always wins. Returns an MCP image block plus dimensions, encoded byte count, and SHA-256 metadata. Never use execution tools to bypass a disabled capture capability.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<CaptureMetadataOutput>()
    )]
    fn capture_screen(&self, Parameters(a): Parameters<CaptureScreenArgs>) -> CallToolResult {
        let snapshot = self.agent_policy.snapshot();
        let captured: Result<CapturedImage, CoreError> = (|| {
            if self.core.pause.is_paused() {
                return Err(CoreError::new(
                    ErrorCode::AccessPaused,
                    "access is paused by the user",
                ));
            }
            if !snapshot.screen_capture_enabled {
                return Err(CoreError::new(
                    ErrorCode::ScreenCaptureDisabled,
                    "screen capture is disabled by the user",
                ));
            }

            let allowed_targets = filter_targets_for_scope(
                self.capture_provider.list_targets()?,
                snapshot.capture_scope,
            );
            if !allowed_targets
                .iter()
                .any(|target| target.id == a.target_id)
            {
                return Err(CoreError::new(
                    ErrorCode::CaptureTargetNotFound,
                    format!(
                        "capture target '{}' is not available in the current scope",
                        a.target_id
                    ),
                ));
            }
            self.capture_provider.capture(&a.target_id, a.max_edge)
        })();

        let decision = match &captured {
            Ok(_) => "allow",
            Err(e)
                if matches!(
                    e.code,
                    ErrorCode::AccessPaused | ErrorCode::ScreenCaptureDisabled
                ) =>
            {
                "deny"
            }
            Err(_) => "error",
        };
        let detail = match &captured {
            Ok(image) => Some(serde_json::json!({
                "target_id": image.target.id,
                "target_kind": image.target.kind,
                "original_width": image.original_width,
                "original_height": image.original_height,
                "returned_width": image.returned_width,
                "returned_height": image.returned_height,
                "encoded_bytes": image.png_bytes.len(),
                "sha256": lc_core::ops::sha256_hex(&image.png_bytes),
            })),
            Err(_) => Some(serde_json::json!({ "target_id": a.target_id })),
        };
        if let Err(e) = lc_core::audit::record_simple(
            &self.core.db,
            "mcp",
            Some("capture_screen"),
            None,
            "capture_screen",
            decision,
            captured.as_ref().err().map(|e| e.code.as_str()),
            None,
            detail,
        ) {
            return err_result(&e);
        }

        match captured {
            Ok(image) => {
                let sha256 = lc_core::ops::sha256_hex(&image.png_bytes);
                let metadata = CaptureMetadataOutput {
                    target_id: image.target.id.clone(),
                    target_kind: image.target.kind,
                    name: image.target.name.clone(),
                    application: image.target.application.clone(),
                    original_width: image.original_width,
                    original_height: image.original_height,
                    returned_width: image.returned_width,
                    returned_height: image.returned_height,
                    encoded_bytes: image.png_bytes.len(),
                    sha256,
                };
                let json = serde_json::to_value(&metadata).unwrap_or(serde_json::Value::Null);
                let encoded = base64::engine::general_purpose::STANDARD.encode(&image.png_bytes);
                let mut result = CallToolResult::success(vec![
                    ContentBlock::image(encoded, "image/png"),
                    ContentBlock::text(serde_json::to_string_pretty(&metadata).unwrap_or_default()),
                ]);
                result.structured_content = Some(json);
                result
            }
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Discover user-approved workspaces and their Read/Write/Execute permissions. Call this first when workspace_id is not already known, then reuse the returned id for later tools. If access is paused, stop and report the user-controlled blocker.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<Vec<WorkspaceSummary>>()
    )]
    fn list_workspaces(&self, Parameters(_): Parameters<EmptyArgs>) -> CallToolResult {
        // The pause kill switch applies to metadata tools too; like every
        // tool call this writes exactly one audit row (fail-closed).
        let audited: Result<Vec<WorkspaceSummary>, CoreError> = (|| {
            if self.core.pause.is_paused() {
                return Err(CoreError::new(
                    ErrorCode::AccessPaused,
                    "access is paused by the user",
                ));
            }
            let rows = lc_core::workspace_repo::list_workspaces(&self.core.db)?;
            Ok(rows.iter().map(summary_of).collect())
        })();
        let unrestricted = self.core.unrestricted.get();
        let detail = Some(serde_json::json!({
            "count": audited.as_ref().map(|v| v.len()).unwrap_or(0),
            "unrestricted": unrestricted,
        }));
        let decision = match &audited {
            Ok(_) => "allow",
            Err(e) if e.code == ErrorCode::AccessPaused => "deny",
            Err(_) => "error",
        };
        if let Err(e) = lc_core::audit::record_simple(
            &self.core.db,
            "mcp",
            Some("list_workspaces"),
            None,
            "list_workspaces",
            decision,
            audited.as_ref().err().map(|e| e.code.as_str()),
            None,
            detail,
        ) {
            // Fail-closed: an unrecordable call is a failed call.
            return err_result(&e);
        }
        match audited {
            Ok(summaries) => ok_structured(&summaries),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Inspect directory structure inside an approved workspace. Use this when the relevant subtree is known; prefer non-recursive or focused listings before broad recursive scans.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<lc_core::ops::list_dir::ListDirResult>()
    )]
    fn list_directory(&self, Parameters(a): Parameters<ListDirArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "list_directory",
            &a.workspace_id,
            false,
            Some(&a.path),
            Some(serde_json::json!({ "recursive": a.recursive })),
            |_, root| lc_core::ops::list_dir::list_dir(&self.core, root, &a.path, a.recursive),
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Locate files by glob pattern inside an approved workspace. Use this before opening many files when names/extensions can narrow the relevant set.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<lc_core::ops::find_files::FindResult>()
    )]
    fn find_files(&self, Parameters(a): Parameters<FindArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "find_files",
            &a.workspace_id,
            false,
            None,
            Some(serde_json::json!({ "pattern": &a.pattern })),
            |_, root| lc_core::ops::find_files::find_files(&self.core, root, &a.pattern),
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Search literal text or regex across an approved workspace. Prefer this before reading many files when symbols, errors, strings, or references can identify the relevant code; then read only the matching files.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<lc_core::ops::search_text::SearchResult>()
    )]
    fn search_text(&self, Parameters(a): Parameters<SearchArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "search_text",
            &a.workspace_id,
            false,
            a.path_prefix.as_deref(),
            Some(serde_json::json!({ "regex": a.is_regex, "case_sensitive": a.case_sensitive })),
            |_, root| {
                lc_core::ops::search_text::search_text(
                    &self.core,
                    root,
                    &a.query,
                    a.path_prefix.as_deref(),
                    a.case_sensitive,
                    a.is_regex,
                    a.max_results,
                )
            },
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Read a targeted file window and return its sha256 revision. Read an existing file before apply_patch and use the latest returned revision as base_revision; re-read after external/stale changes.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<lc_core::ops::read_file::ReadResult>()
    )]
    fn read_file(&self, Parameters(a): Parameters<ReadArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "read_file",
            &a.workspace_id,
            false,
            Some(&a.path),
            None,
            |_, root| {
                lc_core::ops::read_file::read_file(
                    &self.core,
                    root,
                    &a.path,
                    a.offset_line,
                    a.max_lines,
                )
            },
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Read up to 8 targeted files in one call, with per-file results and revisions. Use this for a small independent batch after search/listing has narrowed the relevant files; avoid broad repository dumps.",
        annotations(read_only_hint = true),
        output_schema = schema_for_output::<Vec<lc_core::ops::read_file::FileResult>>()
    )]
    fn read_files(&self, Parameters(a): Parameters<ReadManyArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "read_files",
            &a.workspace_id,
            false,
            None,
            Some(serde_json::json!({ "file_count": a.paths.len() })),
            |_, root| lc_core::ops::read_file::read_files(&self.core, root, &a.paths),
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Create a genuinely new file inside the workspace; this fails if the path already exists. For an existing file, read it and use apply_patch instead of recreating or overwriting it.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<lc_core::ops::write_file::CreateResult>()
    )]
    fn create_file(&self, Parameters(a): Parameters<CreateArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "create_file",
            &a.workspace_id,
            true,
            Some(&a.path),
            Some(serde_json::json!({ "bytes": a.content.len() })),
            |_, root| lc_core::ops::write_file::create_file(&self.core, root, &a.path, &a.content),
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Patch an existing file by replacing one unique old_string. Requires base_revision from the latest read_file/read_files result. On FILE_CHANGED_SINCE_READ, re-read the file, recompute the patch against the new content, and retry only if still valid.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<lc_core::ops::write_file::PatchResult>()
    )]
    fn apply_patch(&self, Parameters(a): Parameters<PatchArgs>) -> CallToolResult {
        match lc_core::pipeline::run_tool_audited_with_context(
            &self.core,
            "apply_patch",
            &a.workspace_id,
            true,
            Some(&a.path),
            None,
            |_, root| {
                lc_core::ops::write_file::apply_patch(
                    &self.core,
                    root,
                    &lc_core::ops::write_file::PatchArgs {
                        rel: a.path.clone(),
                        old_string: a.old_string.clone(),
                        new_string: a.new_string.clone(),
                        base_revision: a.base_revision.clone(),
                    },
                )
            },
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Run an approved developer command (git, cargo, npm, node, python, ...) inside the workspace. Use this to inspect project state and verify changes with targeted tests/build/lint commands. Requires Execute permission; pass a bare approved program name, never a path or shell command string.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<ExecutionOutput>()
    )]
    fn run_command(&self, Parameters(a): Parameters<RunCommandArgs>) -> CallToolResult {
        let detail_ctx = serde_json::json!({ "program": &a.program });
        match lc_core::pipeline::run_tool_with_capability(
            &self.core,
            "run_command",
            &a.workspace_id,
            RequiredCapability::Execute,
            None,
            Some(detail_ctx),
            |_ws, root| {
                // Note: powershell/pwsh are not on the approved-command list,
                // so the policy layer already rejects them — the only path to
                // PowerShell is run_powershell below.
                let req = crate::execution::policy::RunCommandRequest {
                    program: a.program.clone(),
                    args: a.args.clone(),
                    cwd: a.cwd.clone(),
                    timeout_ms: a.timeout_ms,
                    unrestricted: self.core.unrestricted.get(),
                };
                let result =
                    crate::execution::runner::run_command(root, req).map_err(execution_error)?;
                let audited = Audited::new(ExecutionOutput {
                    exit_code: result.exit_code,
                    stdout: result.stdout,
                    stderr: result.stderr,
                    timed_out: result.timed_out,
                    stdout_truncated: result.stdout_truncated,
                    stderr_truncated: result.stderr_truncated,
                    duration_ms: result.duration_ms,
                })
                .with_detail(serde_json::json!({
                    "program": a.program,
                    "arg_count": a.args.len(),
                    "cwd": a.cwd.as_deref().unwrap_or("."),
                    "exit_code": result.exit_code,
                    "duration_ms": result.duration_ms,
                    "timed_out": result.timed_out,
                    "stdout_truncated": result.stdout_truncated,
                    "stderr_truncated": result.stderr_truncated,
                }));
                Ok(audited)
            },
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }

    #[tool(
        description = "Run a PowerShell script via stdin inside the workspace. Only use when the task genuinely needs PowerShell and run_command cannot express it. Requires FullPowerShell mode (unless the user explicitly enabled unrestricted access); never use this to bypass workspace or execution policy.",
        annotations(read_only_hint = false),
        output_schema = schema_for_output::<ExecutionOutput>()
    )]
    fn run_powershell(&self, Parameters(a): Parameters<RunPowerShellArgs>) -> CallToolResult {
        use lc_core::ops::sha256_hex;
        let script_bytes = a.script.as_bytes();
        let script_sha256 = sha256_hex(script_bytes);
        let detail_ctx = serde_json::json!({ "script_bytes": script_bytes.len() });
        // Unrestricted access skips the FullPowerShell workspace-mode gate.
        let unrestricted = self.core.unrestricted.get();
        let detail_ctx = if unrestricted {
            serde_json::json!({ "script_bytes": script_bytes.len(), "unrestricted": true })
        } else {
            detail_ctx
        };
        match lc_core::pipeline::run_tool_with_capability(
            &self.core,
            "run_powershell",
            &a.workspace_id,
            RequiredCapability::Execute,
            None,
            Some(detail_ctx),
            |ws, root| {
                if !unrestricted
                    && ws.execution_mode != lc_core::models::ExecutionMode::FullPowerShell
                {
                    return Err(CoreError::new(
                        ErrorCode::PowershellDisabled,
                        format!("workspace '{}' does not allow full PowerShell", ws.name),
                    ));
                }
                let req = crate::execution::policy::RunPowerShellRequest {
                    script: a.script.clone(),
                    cwd: a.cwd.clone(),
                    timeout_ms: a.timeout_ms,
                    unrestricted,
                };
                let result = crate::execution::runner::run_power_shell(root, req)
                    .map_err(execution_error)?;
                Ok(Audited::new(ExecutionOutput {
                    exit_code: result.exit_code,
                    stdout: result.stdout,
                    stderr: result.stderr,
                    timed_out: result.timed_out,
                    stdout_truncated: result.stdout_truncated,
                    stderr_truncated: result.stderr_truncated,
                    duration_ms: result.duration_ms,
                })
                .with_detail(serde_json::json!({
                    "script_bytes": script_bytes.len(),
                    "script_sha256": script_sha256,
                    "cwd": a.cwd.as_deref().unwrap_or("."),
                    "exit_code": result.exit_code,
                    "duration_ms": result.duration_ms,
                    "timed_out": result.timed_out,
                    "stdout_truncated": result.stdout_truncated,
                    "stderr_truncated": result.stderr_truncated,
                })))
            },
        ) {
            Ok(r) => ok_structured(&r),
            Err(e) => err_result(&e),
        }
    }
}

#[tool_handler]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for WorkspaceServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_prompts()
                .enable_tools()
                .build(),
        )
        .with_instructions(crate::agent_skills::AGENT_INSTRUCTIONS)
    }
}

/// Bind axum to a free loopback port and serve /mcp. Returns the chosen port.
///
/// rmcp defaults `allowed_hosts` to loopback only (DNS-rebinding defense).
/// Requests arrive through tunnels with the public tunnel hostname in
/// `Host`, so host validation is disabled here — access control stays at
/// the workspace/pipeline layer, and the server still binds loopback only.
pub async fn serve(
    core: Arc<CoreServices>,
    agent_policy: Arc<crate::settings::AgentPolicyState>,
    agent_settings: Arc<crate::settings::AgentSettingsController>,
    browser: Arc<crate::browser::manager::BrowserManager>,
) -> std::io::Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let mut config = StreamableHttpServerConfig::default();
    config.allowed_hosts = vec![]; // empty = Host validation off (see doc comment)
    let capture_provider: Arc<dyn ScreenCaptureProvider> = Arc::new(SystemScreenCaptureProvider);
    let service = StreamableHttpService::new(
        move || {
            Ok(WorkspaceServer::new(
                core.clone(),
                agent_policy.clone(),
                agent_settings.clone(),
                browser.clone(),
                capture_provider.clone(),
            ))
        },
        LocalSessionManager::default().into(),
        config,
    );
    let app = Router::new().nest_service("/mcp", service);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("mcp server error: {e}");
        }
    });
    Ok(port)
}
