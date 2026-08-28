//! MCP tool server: 7 workspace-scoped file tools behind the fail-closed
//! audit pipeline.
//!
//! Note: requests rmcp rejects before reaching a handler (malformed args,
//! unknown tool names) produce a protocol error with no audit row; the
//! one-audit-row-per-request guarantee covers requests that reach these
//! handlers. Flagged for spec Task 15 ruling.

use crate::{
    browser::mcp::{
        LightpandaClickArgs, LightpandaConsoleLogsArgs, LightpandaDetectFormsArgs,
        LightpandaEvaluateArgs, LightpandaExtractArgs, LightpandaFillArgs,
        LightpandaFindElementArgs, LightpandaGetCookiesArgs, LightpandaGetEnvArgs,
        LightpandaGetUrlArgs, LightpandaGotoArgs, LightpandaHoverArgs, LightpandaHtmlArgs,
        LightpandaInteractiveElementsArgs, LightpandaLinksArgs, LightpandaMarkdownArgs,
        LightpandaNodeDetailsArgs, LightpandaPressArgs, LightpandaSaveArgs, LightpandaScrollArgs,
        LightpandaSearchArgs, LightpandaSelectOptionArgs, LightpandaSessionCloseArgs,
        LightpandaSessionListArgs, LightpandaSessionNewArgs, LightpandaSetCheckedArgs,
        LightpandaStructuredDataArgs, LightpandaTreeArgs, LightpandaWaitForScriptArgs,
        LightpandaWaitForSelectorArgs, LightpandaWaitForStateArgs,
    },
    screen_capture::{
        filter_targets_for_scope, CaptureTarget, CapturedImage, ScreenCaptureProvider,
        SystemScreenCaptureProvider,
    },
};
use axum::Router;
use base64::Engine as _;
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
use std::sync::Arc;

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
    browser: Arc<dyn crate::browser::lightpanda::BrowserNativeCaller>,
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

/// Builds the public `isError` result for a valid upstream JSON-RPC tool
/// error, preserving the upstream numeric code and message for the authorized
/// caller. Audit stores only the numeric code, never the raw message.
fn upstream_rpc_result(code: i64, message: &str) -> CallToolResult {
    let payload = serde_json::json!({ "upstream_code": code, "message": message });
    CallToolResult::error(vec![ContentBlock::text(payload.to_string())])
}

/// Writes the one fail-closed audit row for a native browser call and returns
/// the public result. An audit failure fails closed and is never replayed.
fn finish_native(
    core: &CoreServices,
    op: &str,
    outcome: crate::browser::audit::NativeCallOutcome,
    detail: Option<serde_json::Value>,
    public: CallToolResult,
) -> CallToolResult {
    if let Err(audit_error) =
        crate::browser::audit::record_native_browser_call(core, op, outcome, detail)
    {
        return err_result(&audit_error);
    }
    public
}

// The #[tool_router] macro generates an associated `Self::tool_router()`
// consumed by #[tool_handler]; no runtime field is needed.
#[tool_router]
impl WorkspaceServer {
    pub fn new(
        core: Arc<CoreServices>,
        agent_policy: Arc<crate::settings::AgentPolicyState>,
        agent_settings: Arc<crate::settings::AgentSettingsController>,
        browser: Arc<dyn crate::browser::lightpanda::BrowserNativeCaller>,
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

    /// Common path for every `browser_lightpanda_*` handler: pause gate, split
    /// the `session_id` routing metadata, run the navigation URL policy, build
    /// content-free audit metadata, forward to the native caller, and forward
    /// the native result / JSON-RPC error unchanged.
    async fn native_tool_call<T: serde::Serialize>(
        &self,
        public_op: &str,
        native_tool: &'static str,
        args: &T,
    ) -> CallToolResult {
        use crate::browser::http::UpstreamToolResponse;
        use crate::browser::mcp::{
            safe_native_audit_detail, split_session_route, validate_native_navigation,
        };

        if self.core.pause.is_paused() {
            let err = CoreError::new(ErrorCode::AccessPaused, "access is paused by the user");
            let public = err_result(&err);
            return finish_native(
                &self.core,
                public_op,
                crate::browser::audit::NativeCallOutcome::Local(&err),
                None,
                public,
            );
        }
        let (session_id, native_args) = match split_session_route(args) {
            Ok(value) => value,
            Err(err) => {
                let public = err_result(&err);
                return finish_native(
                    &self.core,
                    public_op,
                    crate::browser::audit::NativeCallOutcome::Local(&err),
                    None,
                    public,
                );
            }
        };
        if let Err(err) = validate_native_navigation(native_tool, &native_args) {
            let public = err_result(&err);
            return finish_native(
                &self.core,
                public_op,
                crate::browser::audit::NativeCallOutcome::Local(&err),
                None,
                public,
            );
        }
        let mut detail = safe_native_audit_detail(native_tool, &native_args, session_id.as_deref());
        let response = self
            .browser
            .call_native(native_tool, native_args, session_id.as_deref())
            .await;
        match response {
            Ok(UpstreamToolResponse::Result(result)) => {
                let is_error = result.is_error == Some(true);
                finish_native(
                    &self.core,
                    public_op,
                    crate::browser::audit::NativeCallOutcome::Native { is_error },
                    Some(detail),
                    result,
                )
            }
            Ok(UpstreamToolResponse::JsonRpcError { code, message, .. }) => {
                if let Some(obj) = detail.as_object_mut() {
                    obj.insert("upstream_code".into(), serde_json::json!(code));
                }
                let public = upstream_rpc_result(code, &message);
                finish_native(
                    &self.core,
                    public_op,
                    crate::browser::audit::NativeCallOutcome::UpstreamRpc,
                    Some(detail),
                    public,
                )
            }
            Err(err) => {
                let public = err_result(&err);
                finish_native(
                    &self.core,
                    public_op,
                    crate::browser::audit::NativeCallOutcome::Local(&err),
                    Some(detail),
                    public,
                )
            }
        }
    }

    #[tool(
        description = "Navigate the session to an http(s) URL. Native Lightpanda goto; optional timeout (ms) and waitUntil load state.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_goto(
        &self,
        Parameters(a): Parameters<LightpandaGotoArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_goto", "goto", &a)
            .await
    }

    #[tool(
        description = "Run a web search in the session. Native Lightpanda search; optional timeout (ms).",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_search(
        &self,
        Parameters(a): Parameters<LightpandaSearchArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_search", "search", &a)
            .await
    }

    #[tool(
        description = "Return the current page (or one element, or a navigated URL) as Markdown. Optional selector/backendNodeId/maxBytes/url/timeout.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_markdown(
        &self,
        Parameters(a): Parameters<LightpandaMarkdownArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_markdown", "markdown", &a)
            .await
    }

    #[tool(
        description = "Return the current page (or one element, or a navigated URL) as raw HTML. Optional selector/backendNodeId/url/timeout.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_html(
        &self,
        Parameters(a): Parameters<LightpandaHtmlArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_html", "html", &a)
            .await
    }

    #[tool(
        description = "List links on the current page, optionally navigating to a URL first. Optional url/timeout.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_links(
        &self,
        Parameters(a): Parameters<LightpandaLinksArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_links", "links", &a)
            .await
    }

    #[tool(
        description = "Evaluate a JavaScript script in the page and return the native result. Optional url/timeout/save. The script is passed to Lightpanda unchanged.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_evaluate(
        &self,
        Parameters(a): Parameters<LightpandaEvaluateArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_evaluate", "evaluate", &a)
            .await
    }

    #[tool(
        description = "Extract structured data using a native Lightpanda extraction schema string. Optional save.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_extract(
        &self,
        Parameters(a): Parameters<LightpandaExtractArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_extract", "extract", &a)
            .await
    }

    #[tool(
        description = "Return the accessibility/DOM tree. Optional url/timeout/backendNodeId/maxDepth.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_tree(
        &self,
        Parameters(a): Parameters<LightpandaTreeArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_tree", "tree", &a)
            .await
    }

    #[tool(
        description = "Return details for one backend node id, including a usable CSS selector.",
        annotations(read_only_hint = true)
    )]
    async fn browser_lightpanda_node_details(
        &self,
        Parameters(a): Parameters<LightpandaNodeDetailsArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_node_details", "nodeDetails", &a)
            .await
    }

    #[tool(
        description = "List interactive elements on the page. Optional url/timeout.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_interactive_elements(
        &self,
        Parameters(a): Parameters<LightpandaInteractiveElementsArgs>,
    ) -> CallToolResult {
        self.native_tool_call(
            "browser_lightpanda_interactive_elements",
            "interactiveElements",
            &a,
        )
        .await
    }

    #[tool(
        description = "Extract structured data from the page. Optional url/timeout.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_structured_data(
        &self,
        Parameters(a): Parameters<LightpandaStructuredDataArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_structured_data", "structuredData", &a)
            .await
    }

    #[tool(
        description = "Detect forms and their fields on the page. Optional url/timeout.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_detect_forms(
        &self,
        Parameters(a): Parameters<LightpandaDetectFormsArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_detect_forms", "detectForms", &a)
            .await
    }

    #[tool(
        description = "Click an element by CSS selector or backendNodeId.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_click(
        &self,
        Parameters(a): Parameters<LightpandaClickArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_click", "click", &a)
            .await
    }

    #[tool(
        description = "Fill an input identified by selector or backendNodeId with a value.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_fill(
        &self,
        Parameters(a): Parameters<LightpandaFillArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_fill", "fill", &a)
            .await
    }

    #[tool(
        description = "Scroll the window or one node. Optional backendNodeId/x/y.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_scroll(
        &self,
        Parameters(a): Parameters<LightpandaScrollArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_scroll", "scroll", &a)
            .await
    }

    #[tool(
        description = "Wait until a CSS selector matches. Optional timeout (ms).",
        annotations(read_only_hint = true)
    )]
    async fn browser_lightpanda_wait_for_selector(
        &self,
        Parameters(a): Parameters<LightpandaWaitForSelectorArgs>,
    ) -> CallToolResult {
        self.native_tool_call(
            "browser_lightpanda_wait_for_selector",
            "waitForSelector",
            &a,
        )
        .await
    }

    #[tool(
        description = "Wait until a JavaScript expression is truthy. Optional timeout (ms). The script is passed to Lightpanda unchanged.",
        annotations(read_only_hint = true)
    )]
    async fn browser_lightpanda_wait_for_script(
        &self,
        Parameters(a): Parameters<LightpandaWaitForScriptArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_wait_for_script", "waitForScript", &a)
            .await
    }

    #[tool(
        description = "Wait for a page load state (load/domcontentloaded/networkalmostidle/networkidle/done). Optional timeout (ms).",
        annotations(read_only_hint = true)
    )]
    async fn browser_lightpanda_wait_for_state(
        &self,
        Parameters(a): Parameters<LightpandaWaitForStateArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_wait_for_state", "waitForState", &a)
            .await
    }

    #[tool(
        description = "Hover an element by selector or backendNodeId.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_hover(
        &self,
        Parameters(a): Parameters<LightpandaHoverArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_hover", "hover", &a)
            .await
    }

    #[tool(
        description = "Press a key, optionally targeting an element by selector or backendNodeId.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_press(
        &self,
        Parameters(a): Parameters<LightpandaPressArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_press", "press", &a)
            .await
    }

    #[tool(
        description = "Select an option in a <select> by selector or backendNodeId.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_select_option(
        &self,
        Parameters(a): Parameters<LightpandaSelectOptionArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_select_option", "selectOption", &a)
            .await
    }

    #[tool(
        description = "Set the checked state of a checkbox/radio by selector or backendNodeId. checked defaults to true.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_set_checked(
        &self,
        Parameters(a): Parameters<LightpandaSetCheckedArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_set_checked", "setChecked", &a)
            .await
    }

    #[tool(
        description = "Find an element by ARIA role and/or accessible name.",
        annotations(read_only_hint = true)
    )]
    async fn browser_lightpanda_find_element(
        &self,
        Parameters(a): Parameters<LightpandaFindElementArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_find_element", "findElement", &a)
            .await
    }

    #[tool(
        description = "Return and drain buffered console logs for the session.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_console_logs(
        &self,
        Parameters(a): Parameters<LightpandaConsoleLogsArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_console_logs", "consoleLogs", &a)
            .await
    }

    #[tool(
        description = "Return the current page URL of the session.",
        annotations(read_only_hint = true)
    )]
    async fn browser_lightpanda_get_url(
        &self,
        Parameters(a): Parameters<LightpandaGetUrlArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_get_url", "getUrl", &a)
            .await
    }

    #[tool(
        description = "Return cookies, optionally filtered by url, or all cookies when all=true.",
        annotations(read_only_hint = true)
    )]
    async fn browser_lightpanda_get_cookies(
        &self,
        Parameters(a): Parameters<LightpandaGetCookiesArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_get_cookies", "getCookies", &a)
            .await
    }

    #[tool(
        description = "Read a Lightpanda LP_* environment variable by name.",
        annotations(read_only_hint = true)
    )]
    async fn browser_lightpanda_get_env(
        &self,
        Parameters(a): Parameters<LightpandaGetEnvArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_get_env", "getEnv", &a)
            .await
    }

    #[tool(
        description = "Save a PandaScript to the runtime's script directory under a relative path.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_save(
        &self,
        Parameters(a): Parameters<LightpandaSaveArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_save", "save", &a)
            .await
    }

    #[tool(
        description = "Create a new isolated Lightpanda browsing session, optionally named.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_session_new(
        &self,
        Parameters(a): Parameters<LightpandaSessionNewArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_session_new", "session_new", &a)
            .await
    }

    #[tool(
        description = "List active Lightpanda sessions.",
        annotations(read_only_hint = true)
    )]
    async fn browser_lightpanda_session_list(
        &self,
        Parameters(a): Parameters<LightpandaSessionListArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_session_list", "session_list", &a)
            .await
    }

    #[tool(
        description = "Close a named Lightpanda session. The default session cannot be closed.",
        annotations(read_only_hint = false)
    )]
    async fn browser_lightpanda_session_close(
        &self,
        Parameters(a): Parameters<LightpandaSessionCloseArgs>,
    ) -> CallToolResult {
        self.native_tool_call("browser_lightpanda_session_close", "session_close", &a)
            .await
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
    browser: Arc<dyn crate::browser::lightpanda::BrowserNativeCaller>,
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
